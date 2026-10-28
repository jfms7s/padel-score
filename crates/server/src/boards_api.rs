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
//! - `POST /api/boards/{token}/games/{game_id}/leave|give|take` with `{version}` (the game's, from
//!   the view the player saw; a stale one is `409 game_changed`) is 200 [`BoardView`].
//! - `POST /api/boards/{token}/games/{game_id}/swap` with `{version, other_game_id,
//!   other_user_id}` asks a player of an overlapping game to trade places, and `.../time` with
//!   `{version, start, end, agreed}` moves the game (at once when `agreed`, else as a request
//!   the other players answer). Both are 200 [`BoardView`].
//! - `POST /api/boards/{token}/games/{game_id}/start`, with no body, is 200 [`StartPlayers`]: the
//!   game's four players as the caller's own player entries, for New match. It is allowed from
//!   30 minutes before the game until its end; `not_yet`, `game_over`, `game_short`,
//!   `already_started` or `board_over` otherwise, and 404 for a non-player.
//! - `POST /api/board-requests/{id}/accept` and `.../decline`, with no body, answer a request
//!   addressed to the caller: 200 [`BoardView`] of its board. A request that went stale or no
//!   longer works is `409 request_expired` (or the failing code).
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
use crate::starting::StartPlayers;

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
        .route(
            "/api/boards/{token}/games/{game_id}/leave",
            post(leave_game),
        )
        .route("/api/boards/{token}/games/{game_id}/give", post(give_place))
        .route("/api/boards/{token}/games/{game_id}/take", post(take_place))
        .route(
            "/api/boards/{token}/games/{game_id}/start",
            post(start_game),
        )
        .route("/api/boards/{token}/games/{game_id}/swap", post(ask_swap))
        .route("/api/boards/{token}/games/{game_id}/time", post(ask_time))
        .route("/api/board-requests/{id}/accept", post(accept_request))
        .route("/api/board-requests/{id}/decline", post(decline_request))
        .route("/api/me/boards", get(mine))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// A free-time range `[start, end)` in epoch seconds.
#[derive(Debug, Deserialize)]
struct RangeBody {
    start: i64,
    end: i64,
}

/// The body of a game action: the game's `version` from the view the player saw.
#[derive(Debug, Deserialize)]
struct VersionBody {
    version: i64,
}

#[derive(Debug, Deserialize)]
struct SwapBody {
    version: i64,
    other_game_id: String,
    other_user_id: String,
}

#[derive(Debug, Deserialize)]
struct TimeBody {
    version: i64,
    start: i64,
    end: i64,
    /// The players settled it in the chat: apply at once and only tell them.
    #[serde(default)]
    agreed: bool,
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

async fn leave_game(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<VersionBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .leave_game(&token, &game_id, &user.user.id, body.version, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn give_place(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<VersionBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .give_place(&token, &game_id, &user.user.id, body.version, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn take_place(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<VersionBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .take_place(&token, &game_id, &user.user.id, body.version, now())
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn start_game(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
) -> Result<Json<StartPlayers>, ApiError> {
    let players = state
        .store
        .start_game(&token, &game_id, &user.user.id, now())
        .await?;
    Ok(Json(players))
}

async fn ask_swap(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<SwapBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .ask_swap(
            &token,
            &game_id,
            &user.user.id,
            body.version,
            &body.other_game_id,
            &body.other_user_id,
            now(),
        )
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn ask_time(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((token, game_id)): ApiPath<(String, String)>,
    ApiJson(body): ApiJson<TimeBody>,
) -> Result<Json<BoardView>, ApiError> {
    let view = state
        .store
        .ask_time(
            &token,
            &game_id,
            &user.user.id,
            body.version,
            body.start,
            body.end,
            body.agreed,
            now(),
        )
        .await?;
    state.push.wake();
    Ok(Json(view))
}

async fn accept_request(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<BoardView>, ApiError> {
    answer(&state, &user.user.id, &id, true).await
}

async fn decline_request(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<BoardView>, ApiError> {
    answer(&state, &user.user.id, &id, false).await
}

async fn answer(
    state: &AppState,
    user: &str,
    request_id: &str,
    accept: bool,
) -> Result<Json<BoardView>, ApiError> {
    let result = state
        .store
        .answer_request(request_id, user, accept, now())
        .await;
    // An expired answer still commits (the request is closed), and may have queued nothing,
    // but waking costs nothing.
    state.push.wake();
    Ok(Json(result?))
}
