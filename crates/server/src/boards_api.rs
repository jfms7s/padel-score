//! The game board routes, all `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]) because the link token is the board's secret:
//!
//! - `POST /api/boards` with a [`NewBoard`] is 201 [`BoardView`].
//! - `GET /api/boards/{token}` is the view. Anyone with the link may read it; signed out, `me`
//!   is `null`. A device token reads as its user.
//! - `PATCH /api/boards/{token}` with a [`BoardChange`] is 200 (creator only).
//!   `DELETE /api/boards/{token}` is 204 (creator only).
//! - `POST /api/boards/{token}/join` is 200; `POST .../leave` is 204.
//! - `POST /api/boards/{token}/ranges` and `.../ranges/remove` with `{start, end}` are 200.
//! - `GET /api/me/boards` is `{ "items": [BoardOut] }`.
//!
//! Everything but the read takes a signed-in session (a device token is 403). Store conflicts
//! are `409 {"error": code}`. The token is never logged: routes log their template.
//! Writes that can queue notices wake the push sender after they commit.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::{OptionalUser, SessionUser};
use crate::boards::{BoardChange, BoardOut, BoardView, NewBoard};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/boards", post(create))
        .route(
            "/api/boards/{token}",
            get(read).patch(update).delete(delete),
        )
        .route("/api/boards/{token}/join", post(join))
        .route("/api/boards/{token}/leave", post(leave))
        .route("/api/boards/{token}/ranges", post(add_range))
        .route("/api/boards/{token}/ranges/remove", post(remove_range))
        .route("/api/me/boards", get(mine))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// A free-time range `[start, end)` in epoch seconds.
#[derive(Debug, Deserialize)]
struct RangeBody {
    start: i64,
    end: i64,
}

#[derive(Debug, Serialize)]
struct MyBoards {
    items: Vec<BoardOut>,
}

async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(new): ApiJson<NewBoard>,
) -> Result<(StatusCode, Json<BoardView>), ApiError> {
    let view = state.store.create_board(&user.user.id, new, now()).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn read(
    State(state): State<AppState>,
    OptionalUser(user): OptionalUser,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<BoardView>, ApiError> {
    let viewer = user.as_ref().map(|u| u.user.id.as_str());
    let view = state
        .store
        .board_view(&token, viewer, now())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(view))
}

async fn update(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
    ApiJson(change): ApiJson<BoardChange>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .update_board(&token, &user.user.id, change, now())
        .await?;
    Ok(Json(view))
}

async fn delete(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .delete_board(&token, &user.user.id, now())
        .await?;
    state.push.wake();
    Ok(StatusCode::NO_CONTENT)
}

async fn join(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state.store.join_board(&token, &user.user.id, now()).await?;
    Ok(Json(view))
}

async fn leave(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .leave_board(&token, &user.user.id, now())
        .await?;
    state.push.wake();
    Ok(StatusCode::NO_CONTENT)
}

async fn add_range(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
    ApiJson(range): ApiJson<RangeBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .add_free(&token, &user.user.id, range.start, range.end, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn remove_range(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
    ApiJson(range): ApiJson<RangeBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .remove_free(&token, &user.user.id, range.start, range.end, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn mine(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<MyBoards>, ApiError> {
    let items = state.store.my_boards(&user.user.id, now()).await?;
    Ok(Json(MyBoards { items }))
}
