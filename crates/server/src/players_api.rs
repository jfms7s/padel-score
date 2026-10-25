//! The player and history routes, for a signed-in session user (a device token is 403):
//!
//! - `GET /api/players` is the caller's players, archived included: `{ "items": [Player] }`.
//! - `POST /api/players` `{name}` is 201 with the new `Player`, or 200 with the existing one
//!   when the name (case-insensitive) is already taken.
//! - `PATCH /api/players/{id}` `{name?, archived?}` is 200 with the `Player`, 404 for an unknown
//!   id or another user's, 409 `player_exists` on a name clash.
//! - `GET /api/players/{id}?before=&limit=` is the player page:
//!   `{ "player": Player, "placings": [Placing], "items": [PlayerHistoryEntry], "next" }`, or
//!   404 for an unknown id or another user's. An archived player's page still reads.
//! - `GET /api/me/history?before=&limit=` is the caller's finished matches:
//!   `{ "items": [HistoryEntry], "next" }`.
//!
//! Every `Player` carries its `record` (`{wins, losses, draws}`). History pages are newest
//! first; `next` is the `before` of the following page, or `null` on the last one. `limit` is
//! clamped to 1..=100 and defaults to 30; a malformed `before` or a non-numeric `limit` is 400.
//! A blank or over-long name is 422 `invalid_player`. Every response carries
//! `Cache-Control: no-store` ([`accounts::secret_headers`]): history holds spectator tokens.

use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::players::{Player, PlayerChange};
use crate::results::{
    Cursor, DEFAULT_PAGE, HistoryEntry, MAX_PAGE, Placing, PlayerHistoryEntry, Record,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/players", get(list).post(create))
        .route("/api/players/{id}", get(page).patch(update))
        .route("/api/me/history", get(history))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// A `Player` as served: the stored player plus its record.
#[derive(Debug, Serialize)]
struct PlayerWithRecord {
    #[serde(flatten)]
    player: Player,
    record: Record,
}

#[derive(Debug, Serialize)]
struct PlayerList {
    items: Vec<PlayerWithRecord>,
}

#[derive(Debug, Deserialize)]
struct CreatePlayer {
    name: String,
}

/// `?before=&limit=` as sent; an empty value is the same as none.
#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    before: Option<String>,
    limit: Option<String>,
}

struct Page {
    before: Option<Cursor>,
    limit: u32,
}

impl PageQuery {
    fn parse(self) -> Result<Page, ApiError> {
        let before = match self.before.as_deref() {
            None | Some("") => None,
            Some(text) => Some(Cursor::parse(text).ok_or(ApiError::BadRequest)?),
        };
        let limit = match self.limit.as_deref() {
            None | Some("") => DEFAULT_PAGE,
            Some(text) => {
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ApiError::BadRequest);
                }
                // All digits: a value too large for u32 is simply "as many as allowed".
                text.parse::<u32>()
                    .map_or(MAX_PAGE, |n| n.clamp(1, MAX_PAGE))
            }
        };
        Ok(Page { before, limit })
    }
}

/// The page query, with any rejection (e.g. a repeated key) as a JSON 400.
fn page_of(query: Result<Query<PageQuery>, QueryRejection>) -> Result<Page, ApiError> {
    let Ok(Query(query)) = query else {
        return Err(ApiError::BadRequest);
    };
    query.parse()
}

#[derive(Debug, Serialize)]
struct HistoryPage {
    items: Vec<HistoryEntry>,
    next: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlayerPage {
    player: PlayerWithRecord,
    placings: Vec<Placing>,
    items: Vec<PlayerHistoryEntry>,
    next: Option<String>,
}

async fn with_record(
    state: &AppState,
    owner: &str,
    player: Player,
) -> Result<PlayerWithRecord, ApiError> {
    let record = state.store.record(owner, &player.id).await?;
    Ok(PlayerWithRecord { player, record })
}

async fn list(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<PlayerList>, ApiError> {
    let owner = &user.user.id;
    let players = state.store.list_players(owner).await?;
    let records = state.store.records(owner).await?;
    let items = players
        .into_iter()
        .map(|player| {
            let record = records.get(&player.id).copied().unwrap_or_default();
            PlayerWithRecord { player, record }
        })
        .collect();
    Ok(Json(PlayerList { items }))
}

async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(req): ApiJson<CreatePlayer>,
) -> Result<(StatusCode, Json<PlayerWithRecord>), ApiError> {
    let owner = &user.user.id;
    let (player, created) = state
        .store
        .create_or_get_player(owner, &req.name, now())
        .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    let player = if created {
        PlayerWithRecord {
            player,
            record: Record::default(),
        }
    } else {
        with_record(&state, owner, player).await?
    };
    Ok((status, Json(player)))
}

async fn update(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(change): ApiJson<PlayerChange>,
) -> Result<Json<PlayerWithRecord>, ApiError> {
    let owner = &user.user.id;
    let player = state.store.update_player(owner, &id, change).await?;
    Ok(Json(with_record(&state, owner, player).await?))
}

async fn page(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<PlayerPage>, ApiError> {
    let page = page_of(query)?;
    let owner = &user.user.id;
    let player = state
        .store
        .player(owner, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (items, next) = state
        .store
        .player_history(owner, &player.id, page.before.as_ref(), page.limit)
        .await?
        .ok_or(ApiError::NotFound)?;
    let placings = state.store.placings(owner, &player.id).await?;
    Ok(Json(PlayerPage {
        player: with_record(&state, owner, player).await?,
        placings,
        items,
        next: next.map(|c| c.to_string()),
    }))
}

async fn history(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<HistoryPage>, ApiError> {
    let page = page_of(query)?;
    let (items, next) = state
        .store
        .history(&user.user.id, page.before.as_ref(), page.limit)
        .await?;
    Ok(Json(HistoryPage {
        items,
        next: next.map(|c| c.to_string()),
    }))
}
