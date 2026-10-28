//! The owner's routes for scheduled matches, all `Cache-Control: no-store` and
//! `Referrer-Policy: no-referrer` ([`accounts::secret_headers`]): the join token is a secret.
//! Joining and leaving are on `/api/join/{token}` ([`crate::signups_api`]).
//!
//! - `POST /api/scheduled` with a [`NewScheduled`] is 201 [`ScheduledView`].
//! - `PATCH /api/scheduled/{id}` with a [`ScheduledChange`] and `POST .../cancel` are 200
//!   [`ScheduledView`]; `POST .../remove` with `{slot}` too.
//! - `POST /api/scheduled/{id}/start`, with no body, is 200 [`StartPlayers`] for New match.
//!
//! All take a signed-in session; another user's match is 404. An edit or start of a match that
//! is cancelled or started is `409 match_closed`. Notices wake the push sender after they
//! commit. The id is not a secret but the response carries the join token.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{patch, post};
use serde::Deserialize;

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::scheduled::{NewScheduled, ScheduledChange, ScheduledView};
use crate::starting::StartPlayers;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/scheduled", post(create))
        .route("/api/scheduled/{id}", patch(update))
        .route("/api/scheduled/{id}/cancel", post(cancel))
        .route("/api/scheduled/{id}/start", post(start))
        .route("/api/scheduled/{id}/remove", post(remove))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

#[derive(Debug, Deserialize)]
struct RemoveBody {
    slot: u8,
}

async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(new): ApiJson<NewScheduled>,
) -> Result<(StatusCode, Json<ScheduledView>), ApiError> {
    let view = state
        .store
        .create_scheduled(&user.user.id, new, now())
        .await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn update(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(change): ApiJson<ScheduledChange>,
) -> Result<Json<ScheduledView>, ApiError> {
    let view = state
        .store
        .update_scheduled(&user.user.id, &id, change, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn cancel(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<ScheduledView>, ApiError> {
    let view = state
        .store
        .cancel_scheduled(&user.user.id, &id, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn remove(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<RemoveBody>,
) -> Result<Json<ScheduledView>, ApiError> {
    let view = state
        .store
        .remove_from_scheduled(&user.user.id, &id, body.slot, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn start(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<StartPlayers>, ApiError> {
    let players = state
        .store
        .start_scheduled(&user.user.id, &id, now())
        .await?;
    Ok(Json(players))
}
