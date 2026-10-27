//! The match API and its live SSE streams.
//!
//! - `POST /api/matches` creates a match: 201 `{id, scorer_token, spectator_token}` with
//!   `Location: /s/{scorer_token}`. A signed-in caller owns it (see [`crate::owned`]); anyone
//!   else creates an anonymous quick match.
//! - `GET /api/score/{token}` and `GET /api/view/{token}` return the `MatchView`.
//! - `POST /api/score/{token}/commands`, `/replace` and `/final` (a typed final score) run the
//!   command rules; their status and body come from [`CommandOutcome::to_http`] alone.
//! - `PUT /api/score/{token}/start` chooses who serves first, while the log is empty.
//! - `DELETE /api/score/{token}` deletes a match started by mistake: 204, or 409 `finished` /
//!   `event_match` ([`delete_unfinished`]). Its streams end and the owner's list drops it.
//! - The coordinator's `/api/events*` routes are in [`crate::events_api`].
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
use axum::routing::{get, post, put};
use futures_util::StreamExt;
use padel_scoring::{Format, Start};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::auth::OptionalUser;
use crate::commands::{
    Authority, CommandOutcome, CommandRequest, FinalRequest, MatchView, ReplaceRequest,
    apply_command, delete_unfinished, record_final, replace_from, set_start, view_of,
};
use crate::competitions::view::event_id_of_match;
use crate::hub::Subscription;
use crate::players::PlayerSlots;
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
        .route("/api/score/{token}", get(score).delete(delete_match))
        .route("/api/view/{token}", get(view))
        .route("/api/score/{token}/commands", post(commands))
        .route("/api/score/{token}/replace", post(replace))
        .route("/api/score/{token}/final", post(final_score))
        .route("/api/score/{token}/start", put(start))
        .route("/api/score/{token}/stream", get(score_stream))
        .route("/api/view/{token}/stream", get(view_stream))
        .merge(crate::accounts::routes())
        .merge(crate::owned::routes())
        .merge(crate::players_api::routes())
        .merge(crate::linking_api::routes())
        .merge(crate::device::routes())
        .merge(crate::events_api::routes())
        .merge(crate::court_api::routes())
        .merge(crate::tournament_api::routes())
        .merge(crate::non_stop_api::routes())
        .merge(crate::push_api::routes())
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
    /// 403 `history_private`: the player's combined history is not shared with the caller.
    HistoryPrivate,
    /// 422 `invalid_player`: a player name that is blank or too long.
    InvalidPlayer,
    /// 422 `invalid_score`: a typed final score the engine refuses, with the reason.
    InvalidScore(padel_scoring::InvalidScore),
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
            StoreError::InvalidPlayers(msg) => ApiError::InvalidMatch(msg.to_owned()),
            StoreError::InvalidPlayerName => ApiError::InvalidPlayer,
            StoreError::Conflict(code) => ApiError::Conflict(code),
            StoreError::InvalidField(field) => ApiError::Validation { field },
            StoreError::InvalidScore(e) => ApiError::InvalidScore(e),
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
            ApiError::HistoryPrivate => {
                (StatusCode::FORBIDDEN, json!({ "error": "history_private" }))
            }
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
            ApiError::InvalidPlayer => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "invalid_player" }),
            ),
            ApiError::InvalidScore(reason) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "invalid_score", "reason": reason }),
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
    #[serde(default)]
    pub players: PlayerSlots,
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
        players: req.players,
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

/// Delete the match, then end its streams and tell the owner's list streams. Spawned, like
/// [`run_detached`], so a client that goes away after the commit cannot skip the clean-up.
async fn delete_match(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    let task_state = state.clone();
    tokio::spawn(async move {
        delete_unfinished(&task_state.store, &m).await?;
        task_state.hub.close(&m.id);
        if let Some(owner) = &m.owner_user_id {
            task_state.hub.notify_user(owner);
        }
        Ok::<_, StoreError>(())
    })
    .await
    .map_err(internal)??;
    Ok(StatusCode::NO_CONTENT)
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
    run_command(&state, m, req, Authority::Scorer).await
}

/// Apply a command request to `m` and serve the outcome: the one path behind both the scorer's
/// route and the owner's (`/api/me/matches/{id}/commands`), so their shapes and idempotency
/// are the same. `authority` is `Scorer` from the scorer's route and `Owner` from the owner's,
/// which a coordinator's lock does not stop.
pub(crate) async fn run_command(
    state: &AppState,
    m: StoredMatch,
    req: CommandRequest,
    authority: Authority,
) -> Result<Response, ApiError> {
    let store = state.store.clone();
    let (match_id, owner) = (m.id.clone(), m.owner_user_id.clone());
    run_detached(state, match_id, owner, async move {
        apply_command(&store, &m, req, now(), authority).await
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
        replace_from(&store, &m, req, now(), Authority::Scorer).await
    })
    .await
}

/// The court phone's typed final score. Unlike the coordinator's, its `base_revision` is
/// required: a typed score must not overwrite points it has not seen.
#[derive(Debug, Deserialize)]
struct ScorerFinalRequest {
    client_id: String,
    base_revision: i64,
    score: padel_scoring::FinalScore,
}

async fn final_score(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
    ApiJson(req): ApiJson<ScorerFinalRequest>,
) -> Result<Response, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    let req = FinalRequest {
        client_id: req.client_id,
        base_revision: Some(req.base_revision),
        score: req.score,
    };
    let store = state.store.clone();
    let (match_id, owner) = (m.id.clone(), m.owner_user_id.clone());
    run_detached(&state, match_id, owner, async move {
        record_final(&store, &m, req, Authority::Scorer, now()).await
    })
    .await
}

#[derive(Debug, Deserialize)]
struct StartRequest {
    start: Start,
}

/// Choose who serves first. 200 with the view, published like a command's; 409
/// `already_started` once the log has a row; 409 `locked` with the view on a locked match,
/// the same body a command gets.
async fn start(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
    ApiJson(req): ApiJson<StartRequest>,
) -> Result<Response, ApiError> {
    let m = by_scorer(&state.store, &token).await?;
    let task_state = state.clone();
    // Spawned for the same drop safety as `run_detached`: a commit is always published.
    let result = tokio::spawn(async move {
        let result = set_start(&task_state.store, &m, req.start, Authority::Scorer, now()).await;
        if let Ok(view) = &result {
            notify_changed(&task_state, &m.id, m.owner_user_id.as_deref()).await;
            task_state.hub.publish(view.clone());
        }
        result.map_err(|e| (e, m.id))
    })
    .await
    .map_err(internal)?;
    match result {
        Ok(view) => Ok(Json(view).into_response()),
        Err((StoreError::Conflict("locked"), match_id)) => respond(&CommandOutcome::Locked(
            current_view(&state.store, &match_id).await?,
        )),
        Err((e, _)) => Err(e.into()),
    }
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
pub(crate) async fn run_detached<F>(
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
/// - Conflict, locked, rejection, invalid, replayed non-200: nothing changed, nothing to
///   publish.
///
/// The owner's list streams, and for an event match its event's streams, are told whenever the
/// match's streams are ([`notify_changed`]): they re-read, which also picks up a match that has
/// just finished.
async fn publish(state: &AppState, match_id: &str, owner: Option<&str>, outcome: &CommandOutcome) {
    let changed = matches!(
        outcome,
        CommandOutcome::Applied(_) | CommandOutcome::Replayed { status: 200, .. }
    );
    if changed {
        notify_changed(state, match_id, owner).await;
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

/// Tell the owner's list streams, and the event's streams when the match is an event match,
/// that the match changed. An anonymous match is never an event match, so only an owned one
/// costs the (indexed) event lookup.
async fn notify_changed(state: &AppState, match_id: &str, owner: Option<&str>) {
    let Some(owner) = owner else {
        return;
    };
    state.hub.notify_user(owner);
    match event_id_of_match(&state.store, match_id).await {
        Ok(Some(event_id)) => state.hub.notify_event(&event_id),
        Ok(None) => {}
        // The change is stored; an event stream misses one frame until its next change.
        Err(e) => tracing::warn!(match_id, error = %e, "cannot look up the match's event"),
    }
}

/// Serve a command outcome: status and body come from [`CommandOutcome::to_http`] alone.
pub(crate) fn respond(outcome: &CommandOutcome) -> Result<Response, ApiError> {
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
