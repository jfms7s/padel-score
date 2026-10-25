//! The player routes, for a signed-in session user (a device token is 403):
//!
//! - `GET /api/players` is the caller's players, archived included: `{ "items": [Player] }`.
//! - `POST /api/players` `{name}` is 201 with the new `Player`, or 200 with the existing one
//!   when the name (case-insensitive) is already taken.
//! - `PATCH /api/players/{id}` `{name?, archived?}` is 200 with the `Player`, 404 for an unknown
//!   id or another user's, 409 `player_exists` on a name clash.
//!
//! A blank or over-long name is 422 `invalid_player`. Every response carries
//! `Cache-Control: no-store` ([`accounts::secret_headers`]).

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, patch};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::players::{Player, PlayerChange};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/players", get(list).post(create))
        .route("/api/players/{id}", patch(update))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

#[derive(Debug, Serialize)]
struct PlayerList {
    items: Vec<Player>,
}

#[derive(Debug, Deserialize)]
struct CreatePlayer {
    name: String,
}

async fn list(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<PlayerList>, ApiError> {
    let items = state.store.list_players(&user.user.id).await?;
    Ok(Json(PlayerList { items }))
}

async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(req): ApiJson<CreatePlayer>,
) -> Result<(StatusCode, Json<Player>), ApiError> {
    let (player, created) = state
        .store
        .create_or_get_player(&user.user.id, &req.name, now())
        .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(player)))
}

async fn update(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(change): ApiJson<PlayerChange>,
) -> Result<Json<Player>, ApiError> {
    let player = state
        .store
        .update_player(&user.user.id, &id, change)
        .await?;
    Ok(Json(player))
}
