//! The court phone's routes and the event spectator routes. None takes an account: a court
//! route takes one of a court's two tokens, an event-view route the event's spectator token.
//! Every response carries `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]): they carry tokens.
//!
//! - `GET /api/court/{token}` → `CourtState`. The court's scorer token sees the current match's
//!   scorer token; its spectator token does not. Unknown: 404. Once the event is finished the
//!   scorer token is 410 `event_finished`, while the spectator token keeps working.
//! - `GET /api/court/{token}/stream` is SSE: `CourtState` as event `court` at once, then again on
//!   every change to the event (`Hub::notify_event`). A stream opened with the scorer token is a
//!   connected phone ([`CourtPhones`](crate::hub::CourtPhones)) for as long as it is open. It ends
//!   when its token stops matching (the court's scorer token was rotated: the re-read finds no
//!   court) or, for the scorer token, when the event finishes; the reconnect then gets 404 or 410.
//! - `POST /api/court/{token}/advance` moves the court on → `CourtState`: 403 for the spectator
//!   token, 409 `match_not_finished` while the current match is unfinished.
//! - `GET /api/event-view/{token}` → `EventView` (no scorer token anywhere, no phone counts), and
//!   `/stream`, the same as event `event` on every change. Unknown: 404.

use std::convert::Infallible;

use axum::Router;
use axum::extract::State;
use axum::response::sse::Event;
use axum::response::{Json, Response};
use axum::routing::{get, post};
use futures_util::StreamExt;
use serde::Serialize;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::AppState;
use crate::accounts;
use crate::api::{self, ApiError, ApiPath, internal};
use crate::competitions::Status;
use crate::competitions::courts::CourtAccess;
use crate::competitions::view::{
    CourtLookup, CourtState, EventView, court_state_by_token, event_id_by_spectator_token,
    event_view,
};
use crate::hub::{PhoneGuard, Subscription};
use crate::store::StoreError;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/court/{token}", get(court))
        .route("/api/court/{token}/stream", get(court_stream))
        .route("/api/court/{token}/advance", post(advance))
        .route("/api/event-view/{token}", get(view))
        .route("/api/event-view/{token}/stream", get(view_stream))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// The court `token` names: 404 unknown, 410 for the scorer token of a finished event.
async fn lookup(state: &AppState, token: &str) -> Result<CourtLookup, ApiError> {
    let found = court_state_by_token(&state.store, token)
        .await?
        .ok_or(ApiError::NotFound)?;
    if usable(&found) {
        Ok(found)
    } else {
        Err(ApiError::Gone("event_finished"))
    }
}

/// Whether the token still works: the scorer token stops once the event is finished.
fn usable(found: &CourtLookup) -> bool {
    found.access == CourtAccess::Viewer || found.event.status != Status::Finished
}

async fn court(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<CourtState>, ApiError> {
    Ok(Json(lookup(&state, &token).await?.state))
}

async fn advance(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<CourtState>, ApiError> {
    let found = lookup(&state, &token).await?;
    if found.access != CourtAccess::Scorer {
        return Err(ApiError::Forbidden);
    }
    let done = match state.store.advance_court_by_token(&token).await {
        Ok(done) => done,
        // The event finished since the lookup.
        Err(StoreError::Conflict("event_finished")) => {
            return Err(ApiError::Gone("event_finished"));
        }
        Err(e) => return Err(e.into()),
    };
    if let Some(match_id) = &done.rotated {
        state.hub.close(match_id);
    }
    state.hub.notify_event(&found.event.id);
    state.hub.notify_user(&found.event.owner_user_id);
    Ok(Json(lookup(&state, &token).await?.state))
}

async fn court_stream(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Response, ApiError> {
    let found = lookup(&state, &token).await?;
    // Count the phone before subscribing, so its own join does not wake this stream.
    let phone = (found.access == CourtAccess::Scorer).then(|| {
        state
            .court_phones
            .join(&state.hub, &found.event.id, &found.court.id)
    });
    // Subscribe before the first read so no change can fall between the two.
    let subscription = state.hub.subscribe_event(&found.event.id);
    let first = lookup(&state, &token).await?.state;
    let shutdown = state.shutdown.clone().cancelled_owned();
    let watch = Watch {
        subscription,
        event_id: found.event.id,
        pending: Some(first),
        read: CourtRead { state, token },
        _phone: phone,
    };
    let events = futures_util::stream::unfold(watch, next_frame).take_until(shutdown);
    Ok(api::sse(events))
}

async fn view(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<EventView>, ApiError> {
    Ok(Json(
        event_view(&state.store, &token)
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}

async fn view_stream(
    State(state): State<AppState>,
    ApiPath(token): ApiPath<String>,
) -> Result<Response, ApiError> {
    let event_id = event_id_by_spectator_token(&state.store, &token)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Subscribe before the first read so no change can fall between the two.
    let subscription = state.hub.subscribe_event(&event_id);
    let first = event_view(&state.store, &token)
        .await?
        .ok_or(ApiError::NotFound)?;
    let shutdown = state.shutdown.clone().cancelled_owned();
    let watch = Watch {
        subscription,
        event_id,
        pending: Some(first),
        read: ViewRead { state, token },
        _phone: None,
    };
    let events = futures_util::stream::unfold(watch, next_frame).take_until(shutdown);
    Ok(api::sse(events))
}

/// How a stream re-reads its payload; `None` ends the stream.
trait Reread: Send + 'static {
    type Out: Serialize + Send + 'static;
    /// The SSE event name.
    const EVENT: &'static str;
    fn reread(&self) -> impl Future<Output = Option<Self::Out>> + Send;
}

/// A court stream's re-read: by its token, so a rotated scorer token (no court) or a finished
/// event (for the scorer token) ends it.
struct CourtRead {
    state: AppState,
    token: String,
}

impl Reread for CourtRead {
    type Out = CourtState;
    const EVENT: &'static str = "court";

    async fn reread(&self) -> Option<CourtState> {
        let (state, token) = (self.state.clone(), self.token.clone());
        api::detached_reread("court stream", async move {
            Ok(court_state_by_token(&state.store, &token)
                .await?
                .filter(usable)
                .map(|found| found.state))
        })
        .await
        .flatten()
    }
}

/// An event-view stream's re-read: by the spectator token, so a deleted event ends it.
struct ViewRead {
    state: AppState,
    token: String,
}

impl Reread for ViewRead {
    type Out = EventView;
    const EVENT: &'static str = "event";

    async fn reread(&self) -> Option<EventView> {
        let (state, token) = (self.state.clone(), self.token.clone());
        api::detached_reread("event view stream", async move {
            Ok(event_view(&state.store, &token).await?)
        })
        .await
        .flatten()
    }
}

/// One court or event-view stream's state.
struct Watch<R: Reread> {
    subscription: Subscription<()>,
    event_id: String,
    /// A payload to send before waiting on the channel (the first one).
    pending: Option<R::Out>,
    read: R,
    /// A scorer-token court stream's place in the phone count, released when the stream drops.
    _phone: Option<PhoneGuard>,
}

impl<R: Reread> Watch<R> {
    /// Wait for a change to the event, then re-read; `None` ends the stream.
    async fn next_payload(&mut self) -> Option<R::Out> {
        match self.subscription.recv().await {
            Ok(()) => {}
            Err(RecvError::Lagged(skipped)) => {
                tracing::debug!(event_id = %self.event_id, skipped, "event signal stream lagged");
            }
            Err(RecvError::Closed) => return None,
        }
        // Changes that came meanwhile are covered by the one read below.
        while !matches!(
            self.subscription.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Closed)
        ) {}
        self.read.reread().await
    }
}

async fn next_frame<R: Reread>(
    mut watch: Watch<R>,
) -> Option<(Result<Event, Infallible>, Watch<R>)> {
    let payload = match watch.pending.take() {
        Some(payload) => payload,
        None => watch.next_payload().await?,
    };
    match Event::default().event(R::EVENT).json_data(&payload) {
        Ok(event) => Some((Ok(event), watch)),
        Err(e) => {
            internal(format!(
                "cannot encode {} for {}: {e}",
                R::EVENT,
                watch.event_id
            ));
            None
        }
    }
}
