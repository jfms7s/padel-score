//! The match API and its live SSE streams.
//!
//! - `POST /api/matches` creates a match: 201 `{id, scorer_token, spectator_token}` with
//!   `Location: /s/{scorer_token}`. A signed-in caller owns it (see [`crate::owned`]); anyone
//!   else creates an anonymous quick match.
//! - `GET /api/score/{token}` and `GET /api/view/{token}` return the `MatchView`.
//! - `POST /api/score/{token}/commands` and `/replace` run the command rules; their status and
//!   body come from [`CommandOutcome::to_http`] alone.
//! - `GET /api/score/{token}/stream` and `/api/view/{token}/stream` are SSE: the current view as
//!   event `state` at once, then one per applied command.
//!
//! `/api/score/*` takes only the scorer token; `/api/view/*` takes either. Both tokens see the
//! same `MatchView`, which never carries a token. This module is the only place that reads the
//! clock ([`now`]).

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, FromRequestParts, Path, Request, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use padel_scoring::{Format, Start};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::auth::OptionalUser;
use crate::commands::{
    CommandOutcome, CommandRequest, MatchView, ReplaceRequest, apply_command, replace_from, view_of,
};
use crate::hub::Subscription;
use crate::store::{NewMatch, Store, StoreError, StoredMatch, Teams};

/// The largest request body accepted, in bytes.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// How often an idle SSE stream sends a keep-alive comment.
pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// The longest a stream's store re-read may take before the stream ends instead.
pub const REREAD_TIMEOUT: Duration = Duration::from_secs(5);

/// The current time in unix seconds: the one clock read for the whole API.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/matches", post(create_match))
        .route("/api/score/{token}", get(score))
        .route("/api/view/{token}", get(view))
        .route("/api/score/{token}/commands", post(commands))
        .route("/api/score/{token}/replace", post(replace))
        .route("/api/score/{token}/stream", get(score_stream))
        .route("/api/view/{token}/stream", get(view_stream))
        .merge(crate::accounts::routes())
        .merge(crate::owned::routes())
        .merge(crate::device::routes())
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    BadRequest,
    MethodNotAllowed,
    PayloadTooLarge,
    InvalidMatch(String),
    /// 401: no credential, or one that is unknown, expired, revoked or a disabled user's.
    Unauthorized,
    /// 401 `invalid_credentials`: a sign-in or password check failed. One answer for an unknown
    /// email, a wrong password and a disabled user, so none can be told apart.
    InvalidCredentials,
    /// 403: signed in but not allowed (a device token on a session-only route, a non-admin on an
    /// admin route, or a cross-site unsafe request carrying the session cookie).
    Forbidden,
    /// 429 with `Retry-After: {retry_after}` seconds.
    TooMany {
        retry_after: u64,
    },
    /// 409, with the snake_case code from [`StoreError::Conflict`].
    Conflict(&'static str),
    /// 410, with a snake_case code: a secret code (an invite, a reset link) that cannot be used.
    /// One answer for every reason, so a code's state cannot be probed.
    Gone(&'static str),
    /// 422: the named request field fails its policy.
    Validation {
        field: &'static str,
    },
    Internal,
}

/// Log an unexpected failure and hide it behind a 500.
pub(crate) fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %e, "request failed");
    ApiError::Internal
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound => ApiError::NotFound,
            StoreError::InvalidRules(_) | StoreError::InvalidTeams => {
                ApiError::InvalidMatch(e.to_string())
            }
            StoreError::Conflict(code) => ApiError::Conflict(code),
            StoreError::Corrupt(_) | StoreError::Db(_) => internal(e),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, json!({ "error": "not_found" })),
            ApiError::BadRequest => (StatusCode::BAD_REQUEST, json!({ "error": "bad_request" })),
            ApiError::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                json!({ "error": "method_not_allowed" }),
            ),
            ApiError::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({ "error": "payload_too_large" }),
            ),
            ApiError::InvalidMatch(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "invalid_match", "message": message }),
            ),
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, json!({ "error": "unauthorized" }))
            }
            ApiError::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                json!({ "error": "invalid_credentials" }),
            ),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, json!({ "error": "forbidden" })),
            ApiError::TooMany { retry_after } => {
                let body = json!({ "error": "too_many_requests", "retry_after_secs": retry_after });
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, retry_after.to_string())],
                    Json(body),
                )
                    .into_response();
            }
            ApiError::Conflict(code) => (StatusCode::CONFLICT, json!({ "error": code })),
            ApiError::Gone(code) => (StatusCode::GONE, json!({ "error": code })),
            ApiError::Validation { field } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "validation", "field": field }),
            ),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "internal" }),
            ),
        };
        (status, Json(body)).into_response()
    }
}

/// `Json<T>` whose rejection is a JSON body: 413 over the body limit, 400 for anything else
/// (malformed JSON, the wrong shape, a missing content type).
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(json_rejection(&rejection)),
        }
    }
}

fn json_rejection(rejection: &JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::BadRequest
    }
}

/// `Path<T>` whose rejection (e.g. a segment that is not valid UTF-8 once decoded) is a JSON 400.
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| ApiPath(value))
            .map_err(|_| ApiError::BadRequest)
    }
}

/// The JSON 405 for a wrong method on a defined route.
pub async fn method_not_allowed() -> ApiError {
    ApiError::MethodNotAllowed
}

#[derive(Debug, Deserialize)]
pub struct CreateMatchRequest {
    pub format: Format,
    pub start: Start,
    pub teams: Teams,
}

#[derive(Debug, Serialize)]
struct Created<'a> {
    id: &'a str,
    scorer_token: &'a str,
    spectator_token: &'a str,
}

/// A signed-in caller owns the match; anyone else (no credential, or one that fails) creates an
/// anonymous quick match exactly as before, never a 401.
async fn create_match(
    State(state): State<AppState>,
    OptionalUser(user): OptionalUser,
    ApiJson(req): ApiJson<CreateMatchRequest>,
) -> Result<Response, ApiError> {
    let new = NewMatch {
        format: req.format,
        start: req.start,
        teams: req.teams,
        owner_user_id: user.map(|u| u.user.id),
    };
    let m = state.store.create_match(new, now()).await?;
    if let Some(owner) = &m.owner_user_id {
        state.hub.notify_user(owner);
    }
    let location = format!("/s/{}", m.scorer_token);
    let body = Created {
        id: &m.id,
        scorer_token: &m.scorer_token,
        spectator_token: &m.spectator_token,
    };
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(body),
    )
        .into_response())
}

async fn by_scorer(store: &Store, token: &str) -> Result<StoredMatch, ApiError> {
    store
        .by_scorer_token(token)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn by_either(store: &Store, token: &str) -> Result<StoredMatch, ApiError> {
    if let Some(m) = store.by_spectator_token(token).await? {
        return Ok(m);
    }
    by_scorer(store, token).await
}

/// The match's view as stored right now, read in one transaction.
async fn current_view(store: &Store, match_id: &str) -> Result<MatchView, ApiError> {
    let (m, events) = store.snapshot(match_id).await?.ok_or(ApiError::NotFound)?;
    view_of(&m, &events).map_err(|e| internal(format!("match {match_id} does not replay: {e}")))
}

async fn score(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<MatchView>, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    Ok(Json(current_view(&state.store, &m.id).await?))
}

async fn view(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<MatchView>, ApiError> {
    let m = by_either(&state.store, &token).await?;
    Ok(Json(current_view(&state.store, &m.id).await?))
}

async fn commands(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
    ApiJson(req): ApiJson<CommandRequest>,
) -> Result<Response, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    run_command(&state, m, req).await
}

/// Apply a command request to `m` and serve the outcome: the one path behind both the scorer's
/// route and the owner's (`/api/me/matches/{id}/commands`), so their shapes and idempotency
/// are the same.
pub(crate) async fn run_command(
    state: &AppState,
    m: StoredMatch,
    req: CommandRequest,
) -> Result<Response, ApiError> {
    let store = state.store.clone();
    let (match_id, owner) = (m.id.clone(), m.owner_user_id.clone());
    run_detached(state, match_id, owner, async move {
        apply_command(&store, &m, req, now()).await
    })
    .await
}

async fn replace(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
    ApiJson(req): ApiJson<ReplaceRequest>,
) -> Result<Response, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    let store = state.store.clone();
    let (match_id, owner) = (m.id.clone(), m.owner_user_id.clone());
    run_detached(&state, match_id, owner, async move {
        replace_from(&store, &m, req, now()).await
    })
    .await
}

/// Run a command and its publish on their own task, then serve the outcome.
///
/// Drop safety: axum drops a request's future when the client goes away (a phone losing signal
/// mid-tap). If that happened after the command's `tx.commit()` but before the publish, the
/// command would be stored yet never reach the streams, and the client's resend would come back
/// `Replayed`. Spawning makes commit-then-publish run to completion whatever happens to the
/// request; dropping the `JoinHandle` detaches the task, it does not cancel it.
///
/// `owner` is the match's owner, whose list streams hear of the change too.
async fn run_detached<F>(
    state: &AppState,
    match_id: String,
    owner: Option<String>,
    work: F,
) -> Result<Response, ApiError>
where
    F: Future<Output = Result<CommandOutcome, StoreError>> + Send + 'static,
{
    let task_state = state.clone();
    let outcome = tokio::spawn(async move {
        let outcome = work.await?;
        publish(&task_state, &match_id, owner.as_deref(), &outcome).await;
        Ok::<_, StoreError>(outcome)
    })
    .await
    .map_err(internal)??;
    respond(&outcome)
}

/// Tell the match's streams about an outcome that changed (or may have changed) the match.
///
/// - `Applied`: publish the new view.
/// - A replayed 200: the first request applied, but its publish may have been lost (see
///   [`run_detached`]; e.g. a server restart in between). Publish the *current* view as belt and
///   braces; streams drop it by revision if they already have it.
/// - Conflict, rejection, invalid, replayed non-200: nothing changed, nothing to publish.
///
/// The owner's list streams are told whenever the match's streams are: they re-read the list,
/// which also picks up a match that has just finished and drops it.
async fn publish(state: &AppState, match_id: &str, owner: Option<&str>, outcome: &CommandOutcome) {
    let changed = matches!(
        outcome,
        CommandOutcome::Applied(_) | CommandOutcome::Replayed { status: 200, .. }
    );
    if let Some(owner) = owner.filter(|_| changed) {
        state.hub.notify_user(owner);
    }
    match outcome {
        CommandOutcome::Applied(view) => state.hub.publish(view.clone()),
        CommandOutcome::Replayed { status: 200, .. } if state.hub.is_watched(match_id) => {
            // A failed read is already logged by `current_view`; the resend still succeeds.
            if let Ok(view) = current_view(&state.store, match_id).await {
                state.hub.publish(view);
            }
        }
        _ => {}
    }
}

/// Serve a command outcome: status and body come from [`CommandOutcome::to_http`] alone.
fn respond(outcome: &CommandOutcome) -> Result<Response, ApiError> {
    let (status, body) = outcome.to_http().map_err(internal)?;
    let status = StatusCode::from_u16(status).map_err(internal)?;
    Ok((status, [(header::CONTENT_TYPE, "application/json")], body).into_response())
}

async fn score_stream(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Response, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    stream(state, m.id, std::future::pending()).await
}

async fn view_stream(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Response, ApiError> {
    let m = by_either(&state.store, &token).await?;
    stream(state, m.id, std::future::pending()).await
}

/// The SSE stream of one match's views: event `state` with the current view at once, then one
/// per applied command. It ends on shutdown, when the match is purged, or when `end` resolves
/// (a signed-in stream's credential lapsing; the token routes pass a future that never does).
pub(crate) async fn stream(
    state: AppState,
    match_id: String,
    end: impl Future<Output = ()> + Send + 'static,
) -> Result<Response, ApiError> {
    // Subscribe before the first read so no publish can fall between the two.
    let subscription = state.hub.subscribe(&match_id);
    let first = current_view(&state.store, &match_id).await?;
    let watch = Watch {
        subscription,
        store: state.store.clone(),
        match_id,
        pending: Some(Arc::new(first)),
        last_revision: i64::MIN,
    };
    let events = futures_util::stream::unfold(watch, next_event)
        .take_until(until_either(state.shutdown.clone().cancelled_owned(), end));
    Ok(sse(events))
}

/// An SSE response with the streams' keep-alive.
pub(crate) fn sse<S>(events: S) -> Response
where
    S: futures_util::Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response()
}

/// Resolves as soon as either future does.
pub(crate) async fn until_either(
    a: impl Future<Output = ()> + Send,
    b: impl Future<Output = ()> + Send,
) {
    tokio::select! {
        () = a => {}
        () = b => {}
    }
}

/// A stream's store re-read, run on its own task and bounded by [`REREAD_TIMEOUT`]; `None`
/// ends the stream (a failed read, already logged, or one that timed out, logged here).
///
/// Its own task because hyper stops polling the body of a client that is not reading: a read
/// paused inside the body would keep its pool connection (or its place in the pool's queue) for
/// as long as the client stalls. On a task the read finishes, or times out, and lets go of the
/// connection whatever the body does.
pub(crate) async fn detached_reread<T, F>(stream: &'static str, read: F) -> Option<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T, ApiError>> + Send + 'static,
{
    let task = tokio::spawn(tokio::time::timeout(REREAD_TIMEOUT, read));
    match task.await {
        Ok(Ok(result)) => result.ok(),
        Ok(Err(_)) => {
            tracing::warn!(stream, timeout = ?REREAD_TIMEOUT, "stream re-read timed out, ending stream");
            None
        }
        Err(e) => {
            tracing::warn!(stream, error = %e, "stream re-read task failed, ending stream");
            None
        }
    }
}

/// One SSE stream's state.
struct Watch {
    subscription: Subscription,
    store: Store,
    match_id: String,
    /// A view to send before waiting on the channel (the first one).
    pending: Option<Arc<MatchView>>,
    /// The revision last sent. Revisions only grow, so an older or equal view is stale: it was
    /// published before the first read, or overtaken by a later command.
    last_revision: i64,
}

impl Watch {
    /// The next view to send, or `None` to end the stream.
    async fn next_view(&mut self) -> Option<Arc<MatchView>> {
        loop {
            match self.subscription.recv().await {
                Ok(view) if view.revision > self.last_revision => return Some(view),
                Ok(_) => {}
                Err(RecvError::Lagged(skipped)) => {
                    tracing::debug!(match_id = %self.match_id, skipped, "stream lagged, re-reading");
                    // A purged match or a failed read ends the stream; the client reconnects.
                    let (store, match_id) = (self.store.clone(), self.match_id.clone());
                    return detached_reread("match stream", async move {
                        current_view(&store, &match_id).await
                    })
                    .await
                    .map(Arc::new);
                }
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

async fn next_event(mut watch: Watch) -> Option<(Result<Event, Infallible>, Watch)> {
    let view = match watch.pending.take() {
        Some(view) => view,
        None => watch.next_view().await?,
    };
    watch.last_revision = view.revision;
    match Event::default().event("state").json_data(&*view) {
        Ok(event) => Some((Ok(event), watch)),
        Err(e) => {
            tracing::error!(error = %e, match_id = %watch.match_id, "cannot encode view");
            None
        }
    }
}
