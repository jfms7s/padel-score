//! Owned matches: the signed-in user's live list, its stream, and the watch's routes.
//!
//! - `GET /api/me/matches` is the caller's live (not finished) matches, newest activity first,
//!   at most [`LIVE_LIMIT`]: `[{id, label, created_at, last_activity_at, view, scorer_token?,
//!   spectator_token?}]`. The tokens are only for a session caller; a device never gets them.
//! - `GET /api/me/matches/stream` is SSE: that list as event `matches` at once, then again
//!   whenever one of the user's matches is created, scored or finished (the hub's per-user
//!   channel).
//! - `POST /api/me/matches/{id}/commands` is the watch's scoring route: the scorer route's
//!   request, response and idempotency ([`api::run_command`]), for the owner only.
//! - `GET /api/me/matches/{id}/stream` is the scorer stream's `state` frames, for the owner only.
//!
//! Every response carries `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]). Every route takes [`CurrentUser`], so a revoked device or a
//! disabled user is 401 on its next request. A match the caller does not own is 404, as is an unknown one, so existence does not
//! leak. The two streams also re-check their credential every [`AppState::credential_recheck`]
//! (30 s) and end once it no longer holds; the client's reconnect then gets 401.
//!
//! A user may hold at most [`MAX_LIST_STREAMS`] list streams at once; opening another ends
//! their oldest to make room, so a client that leaked streams (a reload, a lost tab) is never
//! locked out. Every change re-reads and replays the list once per stream, so the cap bounds
//! that work. The list stream's re-reads run off the response body
//! ([`api::detached_reread`]), so a client that stops reading cannot pin a pool connection.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::Event;
use axum::response::{Json, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use padel_scoring::Status;
use serde::Serialize;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::task::AbortHandle;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::accounts;
use crate::api::{self, ApiError, ApiJson, ApiPath, internal};
use crate::auth::{AuthVia, Credential, CurrentUser, credential, credential_holds};
use crate::commands::{Authority, CommandRequest, MatchView, view_of};
use crate::hub::Subscription;
use crate::store::StoredMatch;

/// The most matches the live list holds.
pub const LIVE_LIMIT: usize = 20;

/// The most list streams one user may hold open at once.
pub const MAX_LIST_STREAMS: usize = 5;

/// The open list streams per user, oldest first, for [`MAX_LIST_STREAMS`]. The lock guards
/// map operations only, never an await.
#[derive(Debug, Default)]
pub struct ListStreams {
    open: Mutex<ListStreamsInner>,
}

#[derive(Debug, Default)]
struct ListStreamsInner {
    next_id: u64,
    by_user: HashMap<String, VecDeque<(u64, CancellationToken)>>,
}

impl ListStreams {
    pub fn new() -> ListStreams {
        ListStreams::default()
    }

    /// Take one of the user's slots. When they hold [`MAX_LIST_STREAMS`] already, their oldest
    /// is cancelled and its slot handed over at once; that stream ends on its next poll. The
    /// slot is given back when the returned guard drops.
    fn open(self: &Arc<Self>, user_id: &str) -> ListStreamSlot {
        let mut inner = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let id = inner.next_id;
        inner.next_id = inner.next_id.wrapping_add(1);
        let evicted = CancellationToken::new();
        let slots = inner.by_user.entry(user_id.to_owned()).or_default();
        while slots.len() >= MAX_LIST_STREAMS {
            if let Some((_, oldest)) = slots.pop_front() {
                oldest.cancel();
            }
        }
        slots.push_back((id, evicted.clone()));
        ListStreamSlot {
            streams: Arc::clone(self),
            user_id: user_id.to_owned(),
            id,
            evicted,
        }
    }
}

/// One open list stream's slot; dropping it frees the slot (a no-op once it was evicted).
struct ListStreamSlot {
    streams: Arc<ListStreams>,
    user_id: String,
    id: u64,
    /// Cancelled when a newer stream of the same user takes this slot.
    evicted: CancellationToken,
}

impl Drop for ListStreamSlot {
    fn drop(&mut self) {
        let mut inner = self
            .streams
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(slots) = inner.by_user.get_mut(&self.user_id) {
            slots.retain(|(id, _)| *id != self.id);
            if slots.is_empty() {
                inner.by_user.remove(&self.user_id);
            }
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/matches", get(list))
        .route("/api/me/matches/stream", get(list_stream))
        .route("/api/me/matches/{id}/commands", post(commands))
        .route("/api/me/matches/{id}/stream", get(match_stream))
        // The list carries scorer tokens; no cache may keep it, nor any of these.
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// One entry of the live list.
#[derive(Debug, Serialize)]
pub struct OwnedMatch {
    pub id: String,
    /// `"Ana / Rui v João / Marta"`.
    pub label: String,
    pub created_at: i64,
    pub last_activity_at: i64,
    pub view: MatchView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectator_token: Option<String>,
}

fn label(m: &StoredMatch) -> String {
    let [a1, a2] = &m.teams.a;
    let [b1, b2] = &m.teams.b;
    format!("{a1} / {a2} v {b1} / {b2}")
}

/// The caller's live matches. The store hands over the newest candidates by activity that have
/// no stored result; each is replayed here anyway, as a safety net for one finished before the
/// results backfill ran, keeping the first [`LIVE_LIMIT`] still going.
pub async fn live_list(state: &AppState, user: &CurrentUser) -> Result<Vec<OwnedMatch>, ApiError> {
    let with_tokens = user.via == AuthVia::Session;
    let candidates = state.store.live_matches_for_owner(&user.user.id).await?;
    let mut live = Vec::new();
    for candidate in candidates {
        if live.len() >= LIVE_LIMIT {
            break;
        }
        // Read the match and its log together; one purged in between is skipped.
        let Some((m, events)) = state.store.snapshot(&candidate.id).await? else {
            continue;
        };
        let view = match view_of(&m, &events) {
            Ok(view) => view,
            Err(e) => {
                // One broken match must not hide the others.
                tracing::error!(match_id = %m.id, error = %e, "owned match does not replay");
                continue;
            }
        };
        if matches!(view.score.status, Status::Finished { .. }) {
            continue;
        }
        live.push(OwnedMatch {
            id: m.id.clone(),
            label: label(&m),
            created_at: m.created_at,
            last_activity_at: m.last_activity_at,
            view,
            scorer_token: with_tokens.then(|| m.scorer_token.clone()),
            spectator_token: with_tokens.then(|| m.spectator_token.clone()),
        });
    }
    Ok(live)
}

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Vec<OwnedMatch>>, ApiError> {
    Ok(Json(live_list(&state, &user).await?))
}

/// The match, if the caller owns it; 404 otherwise, whether or not it exists.
async fn owned(state: &AppState, user: &CurrentUser, id: &str) -> Result<StoredMatch, ApiError> {
    state
        .store
        .match_owned_by(id, &user.user.id)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn commands(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<CommandRequest>,
) -> Result<Response, ApiError> {
    let m = owned(&state, &user, &id).await?;
    api::run_command(&state, m, req, Authority::Owner).await
}

async fn match_stream(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
) -> Result<Response, ApiError> {
    let m = owned(&state, &user, &id).await?;
    let lapsed = lapsed(&state, &headers, &user)?;
    api::stream(state, m.id, lapsed).await
}

/// A future that resolves once the request's credential no longer signs `user` in, checked
/// every [`AppState::credential_recheck`]. A failed check also resolves it: the stream ends and
/// the client's reconnect decides.
///
/// The checks run on their own task, not inside the response body: hyper stops polling a body
/// whose client is not reading, and a check paused mid-query would hold a pool connection for
/// as long. Dropping the future (the stream ending) aborts the task.
fn lapsed(
    state: &AppState,
    headers: &HeaderMap,
    user: &CurrentUser,
) -> Result<impl Future<Output = ()> + Send + 'static, ApiError> {
    // `CurrentUser` accepted this request, so it carries a credential.
    let cred = credential(headers, &state.config).ok_or(ApiError::Unauthorized)?;
    let done = CancellationToken::new();
    let task = tokio::spawn(watch_credential(
        state.clone(),
        cred,
        user.user.id.clone(),
        done.clone(),
    ));
    let abort = AbortOnDrop(task.abort_handle());
    Ok(async move {
        let _abort = abort;
        done.cancelled().await;
    })
}

/// Aborts its task when dropped.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn watch_credential(
    state: AppState,
    cred: Credential,
    user_id: String,
    done: CancellationToken,
) {
    let mut tick = tokio::time::interval(state.credential_recheck);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // The first tick is immediate, and the request was just authenticated.
    tick.tick().await;
    loop {
        tick.tick().await;
        match credential_holds(&state, &cred, &user_id).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(user_id = %user_id, "credential lapsed, ending stream");
                break;
            }
            Err(e) => {
                tracing::warn!(user_id = %user_id, error = ?e, "credential re-check failed, ending stream");
                break;
            }
        }
    }
    done.cancel();
}

async fn list_stream(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let lapsed = lapsed(&state, &headers, &user)?;
    let slot = state.list_streams.open(&user.user.id);
    let evicted = slot.evicted.clone().cancelled_owned();
    // Subscribe before the first read so no change can fall between the two.
    let subscription = state.hub.subscribe_user(&user.user.id);
    let first = live_list(&state, &user).await?;
    let shutdown = state.shutdown.clone().cancelled_owned();
    let watch = ListWatch {
        subscription,
        state,
        user,
        pending: Some(first),
        _slot: slot,
    };
    let events = futures_util::stream::unfold(watch, next_list).take_until(api::until_either(
        shutdown,
        api::until_either(lapsed, evicted),
    ));
    Ok(api::sse(events))
}

/// One list stream's state.
struct ListWatch {
    subscription: Subscription<()>,
    state: AppState,
    user: CurrentUser,
    /// A list to send before waiting on the channel (the first one).
    pending: Option<Vec<OwnedMatch>>,
    /// Held for as long as the stream lives.
    _slot: ListStreamSlot,
}

impl ListWatch {
    /// Wait for a change, then re-read the list; `None` ends the stream.
    async fn next_list(&mut self) -> Option<Vec<OwnedMatch>> {
        match self.subscription.recv().await {
            Ok(()) => {}
            Err(RecvError::Lagged(skipped)) => {
                tracing::debug!(user_id = %self.user.user.id, skipped, "list stream lagged");
            }
            Err(RecvError::Closed) => return None,
        }
        // Changes that came meanwhile are covered by the one read below.
        while !matches!(
            self.subscription.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Closed)
        ) {}
        // A failed read (already logged) ends the stream; the client reconnects.
        let (state, user) = (self.state.clone(), self.user.clone());
        api::detached_reread("list stream", async move { live_list(&state, &user).await }).await
    }
}

async fn next_list(mut watch: ListWatch) -> Option<(Result<Event, Infallible>, ListWatch)> {
    let list = match watch.pending.take() {
        Some(list) => list,
        None => watch.next_list().await?,
    };
    match Event::default().event("matches").json_data(&list) {
        Ok(event) => Some((Ok(event), watch)),
        Err(e) => {
            internal(format!("cannot encode the owned matches: {e}"));
            None
        }
    }
}
