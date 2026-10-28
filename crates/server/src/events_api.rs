//! The coordinator's event routes and the live event stream. Every route takes a
//! [`SessionUser`] (anonymous is 401, a device token 403, a cross-site unsafe request 403), and
//! every response carries `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]): event details hold scorer tokens. Another user's event, team,
//! court or match is 404, as is an unknown one.
//!
//! - `GET /api/events` `{items:[Event]}`; `POST /api/events` `NewCompetition` → 201 `EventDetail`.
//! - `GET|PATCH|DELETE /api/events/{id}`; `POST /api/events/{id}/start`, `/finish`
//!   (`{places?}`) and `/cancel` (an event that has not started) → `EventDetail`.
//! - `/api/events/{id}/teams[/{team_id}]` → `EventTeam`; `/courts[/{court_id}]` and
//!   `/courts/{court_id}/rotate` → `Court`; `/courts/{court_id}/advance` → `EventDetail`.
//! - `POST /api/events/{id}/matches` and `PATCH /matches/{match_id}` → `EventMatch`;
//!   `POST /matches/{match_id}/result` is the typed result, through the same command path as
//!   `/api/score/{token}/final` with `Authority::Owner` and no `base_revision`.
//! - `GET /api/events/{id}/stream` is SSE: the `EventDetail` as event `event` at once, then
//!   again on every change to the event (`Hub::notify_event`).
//!
//! Every mutating route notifies the event's streams after its commit. A route that rotates a
//! match's scorer token closes that match's hub channel, so streams opened with the old token
//! end.

use std::convert::Infallible;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Event;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, patch, post};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::AppState;
use crate::accounts;
use crate::api::{self, ApiError, ApiJson, ApiPath, internal, now};
use crate::auth::SessionUser;
use crate::commands::{Authority, FinalRequest, record_final};
use crate::competitions::places::Place;
use crate::competitions::view::{
    CourtOut, EventDetail, EventMatch, EventSummary, event_detail, event_id_of_match,
};
use crate::competitions::{CompetitionChange, EventTeam, NewCompetition, TeamChange};
use crate::hub::Subscription;
use crate::store::present;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/events", get(list).post(create))
        .route("/api/events/{id}", get(read).patch(update).delete(remove))
        .route("/api/events/{id}/start", post(start))
        .route("/api/events/{id}/finish", post(finish))
        .route("/api/events/{id}/cancel", post(cancel))
        .route("/api/events/{id}/stream", get(stream))
        .route("/api/events/{id}/teams", post(add_team))
        .route(
            "/api/events/{id}/teams/{team_id}",
            patch(update_team).delete(delete_team),
        )
        .route("/api/events/{id}/courts", post(add_court))
        .route(
            "/api/events/{id}/courts/{court_id}",
            patch(rename_court).delete(delete_court),
        )
        .route(
            "/api/events/{id}/courts/{court_id}/rotate",
            post(rotate_court),
        )
        .route(
            "/api/events/{id}/courts/{court_id}/advance",
            post(advance_court),
        )
        .route("/api/events/{id}/matches", post(create_match))
        .route("/api/events/{id}/matches/{match_id}", patch(update_match))
        .route(
            "/api/events/{id}/matches/{match_id}/result",
            post(match_result),
        )
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// The owner's event detail, or 404.
async fn detail(state: &AppState, owner: &str, id: &str) -> Result<EventDetail, ApiError> {
    event_detail(&state.store, owner, id, &state.court_phones)
        .await?
        .ok_or(ApiError::NotFound)
}

/// Tell the event's streams it changed, then serve its fresh detail. The owner's live list is
/// told too: a match created, moved, advanced or rotated changes what it shows (a court's
/// current matches are in it, with their scorer tokens).
async fn changed(state: &AppState, owner: &str, id: &str) -> Result<EventDetail, ApiError> {
    state.hub.notify_event(id);
    state.hub.notify_user(owner);
    detail(state, owner, id).await
}

fn court_of(detail: &EventDetail, court_id: &str) -> Result<CourtOut, ApiError> {
    detail.court(court_id).cloned().ok_or(ApiError::NotFound)
}

fn match_of(detail: &EventDetail, match_id: &str) -> Result<EventMatch, ApiError> {
    detail
        .event_match(match_id)
        .cloned()
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Serialize)]
struct EventList {
    items: Vec<EventSummary>,
}

async fn list(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<EventList>, ApiError> {
    let items = state
        .store
        .competitions(&user.user.id)
        .await?
        .iter()
        .map(|(ev, teams, courts)| EventSummary::new(ev, *teams, *courts))
        .collect();
    Ok(Json(EventList { items }))
}

async fn create(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(new): ApiJson<NewCompetition>,
) -> Result<Response, ApiError> {
    let owner = &user.user.id;
    let ev = state.store.create_competition(owner, new, now()).await?;
    let detail = detail(&state, owner, &ev.id).await?;
    Ok((StatusCode::CREATED, Json(detail)).into_response())
}

async fn read(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<EventDetail>, ApiError> {
    Ok(Json(detail(&state, &user.user.id, &id).await?))
}

async fn update(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(change): ApiJson<CompetitionChange>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    state
        .store
        .update_competition(owner, &id, change, now())
        .await?;
    state.push.wake();
    Ok(Json(changed(&state, owner, &id).await?))
}

async fn remove(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete_competition(&user.user.id, &id).await?;
    // Its streams re-read, find nothing and end.
    state.hub.notify_event(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn start(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    state.store.start_competition(owner, &id, now()).await?;
    Ok(Json(changed(&state, owner, &id).await?))
}

async fn cancel(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    state.store.cancel_competition(owner, &id, now()).await?;
    state.push.wake();
    Ok(Json(changed(&state, owner, &id).await?))
}

#[derive(Debug, Deserialize)]
struct FinishRequest {
    #[serde(default)]
    places: Option<Vec<Place>>,
}

async fn finish(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<FinishRequest>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    let (_, rotated) = state
        .store
        .finish_competition(owner, &id, req.places, now())
        .await?;
    for match_id in &rotated {
        state.hub.close(match_id);
    }
    Ok(Json(changed(&state, owner, &id).await?))
}

#[derive(Debug, Deserialize)]
struct NewTeam {
    players: Vec<String>,
    #[serde(default)]
    name: Option<String>,
}

/// Exactly two player ids, else 422 `players` (not the 400 of a malformed body).
fn pair(players: Vec<String>) -> Result<[String; 2], ApiError> {
    <[String; 2]>::try_from(players).map_err(|_| ApiError::Validation { field: "players" })
}

async fn add_team(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<NewTeam>,
) -> Result<Response, ApiError> {
    let players = pair(req.players)?;
    let team = state
        .store
        .add_team(&user.user.id, &id, players, req.name, now())
        .await?;
    state.hub.notify_event(&id);
    state.push.wake();
    Ok((StatusCode::CREATED, Json(team)).into_response())
}

#[derive(Debug, Deserialize)]
struct TeamPatch {
    #[serde(default, deserialize_with = "present")]
    name: Option<Option<String>>,
    #[serde(default)]
    players: Option<Vec<String>>,
}

async fn update_team(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, team_id)): ApiPath<(String, String)>,
    ApiJson(req): ApiJson<TeamPatch>,
) -> Result<Json<EventTeam>, ApiError> {
    let change = TeamChange {
        name: req.name,
        players: req.players.map(pair).transpose()?,
    };
    let team = state
        .store
        .update_team(&user.user.id, &id, &team_id, change)
        .await?;
    state.hub.notify_event(&id);
    Ok(Json(team))
}

async fn delete_team(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, team_id)): ApiPath<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .delete_team(&user.user.id, &id, &team_id, now())
        .await?;
    state.hub.notify_event(&id);
    state.push.wake();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct NewCourt {
    #[serde(default)]
    name: Option<String>,
}

async fn add_court(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<NewCourt>,
) -> Result<Response, ApiError> {
    let owner = &user.user.id;
    let court = state.store.add_court(owner, &id, req.name).await?;
    let detail = changed(&state, owner, &id).await?;
    Ok((StatusCode::CREATED, Json(court_of(&detail, &court.id)?)).into_response())
}

#[derive(Debug, Deserialize)]
struct CourtRename {
    name: String,
}

async fn rename_court(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, court_id)): ApiPath<(String, String)>,
    ApiJson(req): ApiJson<CourtRename>,
) -> Result<Json<CourtOut>, ApiError> {
    let owner = &user.user.id;
    state
        .store
        .rename_court(owner, &id, &court_id, req.name)
        .await?;
    let detail = changed(&state, owner, &id).await?;
    Ok(Json(court_of(&detail, &court_id)?))
}

async fn delete_court(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, court_id)): ApiPath<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .delete_court(&user.user.id, &id, &court_id)
        .await?;
    state.hub.notify_event(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn rotate_court(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, court_id)): ApiPath<(String, String)>,
) -> Result<Json<CourtOut>, ApiError> {
    let owner = &user.user.id;
    let court = state.store.rotate_court(owner, &id, &court_id).await?;
    if let Some(match_id) = &court.current_match_id {
        state.hub.close(match_id);
    }
    let detail = changed(&state, owner, &id).await?;
    Ok(Json(court_of(&detail, &court_id)?))
}

async fn advance_court(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, court_id)): ApiPath<(String, String)>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    let done = state.store.advance_court(owner, &id, &court_id).await?;
    if let Some(match_id) = &done.rotated {
        state.hub.close(match_id);
    }
    Ok(Json(changed(&state, owner, &id).await?))
}

#[derive(Debug, Deserialize)]
struct NewEventMatch {
    team_a: String,
    team_b: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    court_id: Option<String>,
}

async fn create_match(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<NewEventMatch>,
) -> Result<Response, ApiError> {
    let owner = &user.user.id;
    let row = state
        .store
        .create_event_match(
            owner,
            &id,
            &req.team_a,
            &req.team_b,
            req.label,
            req.court_id.as_deref(),
            now(),
        )
        .await?;
    let detail = changed(&state, owner, &id).await?;
    Ok((StatusCode::CREATED, Json(match_of(&detail, &row.match_id)?)).into_response())
}

#[derive(Debug, Deserialize)]
struct MatchPatch {
    /// Absent: no move. `null`: off any court.
    #[serde(default, deserialize_with = "present")]
    court_id: Option<Option<String>>,
    #[serde(default)]
    label: Option<String>,
}

async fn update_match(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, match_id)): ApiPath<(String, String)>,
    ApiJson(req): ApiJson<MatchPatch>,
) -> Result<Json<EventMatch>, ApiError> {
    let owner = &user.user.id;
    let court = req.court_id.as_ref().map(Option::as_deref);
    let rotated = state
        .store
        .update_event_match(owner, &id, &match_id, court, req.label)
        .await?;
    if let Some(rotated) = &rotated {
        state.hub.close(rotated);
    }
    let detail = changed(&state, owner, &id).await?;
    Ok(Json(match_of(&detail, &match_id)?))
}

#[derive(Debug, Deserialize)]
struct ResultRequest {
    client_id: String,
    score: padel_scoring::FinalScore,
}

/// The coordinator's typed result: replaces the match's log and locks it against the scorer
/// token. Its response and idempotency are the command path's; the publish tells this event's
/// streams.
async fn match_result(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, match_id)): ApiPath<(String, String)>,
    ApiJson(req): ApiJson<ResultRequest>,
) -> Result<Response, ApiError> {
    let owner = &user.user.id;
    if state.store.competition(owner, &id).await?.is_none()
        || event_id_of_match(&state.store, &match_id).await?.as_deref() != Some(id.as_str())
    {
        return Err(ApiError::NotFound);
    }
    let m = state
        .store
        .match_owned_by(&match_id, owner)
        .await?
        .ok_or(ApiError::NotFound)?;
    let req = FinalRequest {
        client_id: req.client_id,
        base_revision: None,
        score: req.score,
    };
    let store = state.store.clone();
    let owner = m.owner_user_id.clone();
    api::run_detached(&state, m.id.clone(), owner, async move {
        record_final(&store, &m, req, Authority::Owner, now()).await
    })
    .await
}

async fn stream(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    headers: HeaderMap,
    ApiPath(id): ApiPath<String>,
) -> Result<Response, ApiError> {
    // Subscribe before the first read so no change can fall between the two.
    let subscription = state.hub.subscribe_event(&id);
    let first = detail(&state, &user.user.id, &id).await?;
    let lapsed = crate::owned::lapsed(&state, &headers, &user)?;
    let shutdown = state.shutdown.clone().cancelled_owned();
    let watch = EventWatch {
        subscription,
        owner: user.user.id.clone(),
        id,
        state,
        pending: Some(first),
    };
    let events = futures_util::stream::unfold(watch, next_event)
        .take_until(api::until_either(shutdown, lapsed));
    Ok(api::sse(events))
}

/// One event stream's state.
struct EventWatch {
    subscription: Subscription<()>,
    state: AppState,
    owner: String,
    id: String,
    /// A detail to send before waiting on the channel (the first one).
    pending: Option<EventDetail>,
}

impl EventWatch {
    /// Wait for a change, then re-read the detail; `None` ends the stream (a failed read, or
    /// the event is gone).
    async fn next_detail(&mut self) -> Option<EventDetail> {
        match self.subscription.recv().await {
            Ok(()) => {}
            Err(RecvError::Lagged(skipped)) => {
                tracing::debug!(event_id = %self.id, skipped, "event stream lagged");
            }
            Err(RecvError::Closed) => return None,
        }
        // Changes that came meanwhile are covered by the one read below.
        while !matches!(
            self.subscription.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Closed)
        ) {}
        let (state, owner, id) = (self.state.clone(), self.owner.clone(), self.id.clone());
        api::detached_reread("event stream", async move {
            Ok(event_detail(&state.store, &owner, &id, &state.court_phones).await?)
        })
        .await
        .flatten()
    }
}

async fn next_event(mut watch: EventWatch) -> Option<(Result<Event, Infallible>, EventWatch)> {
    let detail = match watch.pending.take() {
        Some(detail) => detail,
        None => watch.next_detail().await?,
    };
    match Event::default().event("event").json_data(&detail) {
        Ok(event) => Some((Ok(event), watch)),
        Err(e) => {
            internal(format!("cannot encode event {}: {e}", watch.id));
            None
        }
    }
}
