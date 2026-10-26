//! The tournament's coordinator routes. Like the event routes, each takes a [`SessionUser`]
//! (anonymous is 401, a device token 403), another user's event, a non-tournament event or an
//! unknown match is 404, and responses carry [`accounts::secret_headers`]: they serve the
//! `EventDetail`, which holds scorer tokens.
//!
//! - `POST /api/events/{id}/bracket/redraw`: draw again before any bracket match has started
//!   (409 `event_not_running`, `play_started`).
//! - `POST /api/events/{id}/bracket/decisions/{match_id}` `{"choice":"replay"|"keep"}`: answer a
//!   held match (409 `no_decision`, `event_finished`; 422 `validation` `choice`).
//! - `POST /api/events/{id}/bracket/fill-courts`: put waiting matches on free courts (409
//!   `event_not_running`).
//!
//! After each, the hub channels of the withdrawn matches are closed (their streams end), the
//! event's streams and the owner's live list are told, and the fresh `EventDetail` is served.

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use serde::Deserialize;

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::competitions::tournament::store::Choice;
use crate::competitions::view::{EventDetail, event_detail};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/events/{id}/bracket/redraw", post(redraw))
        .route(
            "/api/events/{id}/bracket/decisions/{match_id}",
            post(decide),
        )
        .route("/api/events/{id}/bracket/fill-courts", post(fill_courts))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// Close the withdrawn matches' channels, tell the event's and the owner's streams, then serve
/// the owner's fresh detail.
async fn changed(
    state: &AppState,
    owner: &str,
    id: &str,
    withdrawn: &[String],
) -> Result<Json<EventDetail>, ApiError> {
    for match_id in withdrawn {
        state.hub.close(match_id);
    }
    state.hub.notify_event(id);
    state.hub.notify_user(owner);
    event_detail(&state.store, owner, id, &state.court_phones)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn redraw(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    let withdrawn = state.store.redraw_bracket(owner, &id, now()).await?;
    changed(&state, owner, &id, &withdrawn).await
}

/// The body is read loosely so that a missing or unknown choice is a 422 on `choice`, not the
/// 400 of a malformed body.
#[derive(Debug, Deserialize)]
struct DecisionRequest {
    #[serde(default)]
    choice: Option<serde_json::Value>,
}

async fn decide(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, match_id)): ApiPath<(String, String)>,
    ApiJson(req): ApiJson<DecisionRequest>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    let choice: Choice = req
        .choice
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or(ApiError::Validation { field: "choice" })?;
    let withdrawn = state
        .store
        .decide_bracket(owner, &id, &match_id, choice, now())
        .await?;
    changed(&state, owner, &id, &withdrawn).await
}

async fn fill_courts(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    state.store.fill_bracket_courts(owner, &id).await?;
    changed(&state, owner, &id, &[]).await
}
