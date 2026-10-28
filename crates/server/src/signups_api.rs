//! The sign-up routes, all `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]): the join link and the partner link are secrets.
//!
//! - `GET /api/join/{token}` is the [`JoinView`] of the event behind a join link, for anyone with
//!   the link (signed out, `me` is `null`). An unknown link, or an event whose sign-ups are off,
//!   is 404.
//! - `POST /api/join/{token}` with a [`JoinAs`] signs the caller up: 200 [`MyEntry`].
//!   `DELETE /api/join/{token}` leaves: 204.
//! - `GET /api/partner/{ptoken}` is the [`PartnerInvite`]; `POST .../accept` is 200 [`MyEntry`];
//!   `POST .../decline` is 204 (the inviter gets a fresh link).
//! - The organiser's, owner only (another user's event is 404):
//!   `GET /api/events/{id}/signups` is the [`SignupList`]; `DELETE .../signups/{entry}` is 204;
//!   `POST .../signups/{entry}/split` and `POST .../signups/pair` with `{entries: [a, b]}` are
//!   200 with the fresh list.
//!
//! Joining, leaving, accepting and the organiser's changes tell the event's streams and the
//! organiser's desk, and wake the push sender after they commit. Conflicts are
//! `409 {"error": code}`. The tokens are never logged: routes log their template.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::{OptionalUser, SessionUser};
use crate::competitions::signups::{EventJoinInfo, JoinAs, MyEntry, PartnerInvite, SignupList};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/join/{token}", get(read_join).post(join).delete(leave))
        .route("/api/partner/{ptoken}", get(read_invite))
        .route("/api/partner/{ptoken}/accept", post(accept))
        .route("/api/partner/{ptoken}/decline", post(decline))
        .route("/api/events/{id}/signups", get(list))
        .route("/api/events/{id}/signups/pair", post(pair))
        .route("/api/events/{id}/signups/{entry_id}", delete(remove))
        .route("/api/events/{id}/signups/{entry_id}/split", post(split))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// What a join link opens. Task 16 adds a scheduled match.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JoinView {
    Event {
        event: EventJoinInfo,
        list: SignupList,
        me: Option<MyEntry>,
    },
}

#[derive(Debug, Deserialize)]
struct PairBody {
    entries: [String; 2],
}

/// Tell the event's streams and its organiser's desk that the sign-ups changed, and wake the
/// push sender for the notices the change queued.
fn changed(state: &AppState, event_id: &str, owner: &str) {
    state.hub.notify_event(event_id);
    state.hub.notify_user(owner);
    state.push.wake();
}

/// [`changed`] for the event behind a join link.
async fn changed_by_link(state: &AppState, join_token: &str) -> Result<(), ApiError> {
    if let Some((event_id, owner)) = state.store.join_link_event(join_token).await? {
        changed(state, &event_id, &owner);
    }
    Ok(())
}

async fn read_join(
    State(state): State<AppState>,
    OptionalUser(user): OptionalUser,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<JoinView>, ApiError> {
    let viewer = user.as_ref().map(|u| u.user.id.as_str());
    let page = state
        .store
        .join_page(&token, viewer, now())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(JoinView::Event {
        event: page.event,
        list: page.list,
        me: page.me,
    }))
}

async fn join(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
    ApiJson(as_): ApiJson<JoinAs>,
) -> Result<Json<MyEntry>, ApiError> {
    let mine = state
        .store
        .join_event(&token, &user.user.id, as_, now())
        .await?;
    changed_by_link(&state, &token).await?;
    Ok(Json(mine))
}

async fn leave(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .leave_event(&token, &user.user.id, now())
        .await?;
    changed_by_link(&state, &token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn read_invite(
    State(state): State<AppState>,
    OptionalUser(_): OptionalUser,
    ApiPath(ptoken): ApiPath<String>,
) -> Result<Json<PartnerInvite>, ApiError> {
    let invite = state
        .store
        .partner_invite(&ptoken)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(invite))
}

async fn accept(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(ptoken): ApiPath<String>,
) -> Result<Json<MyEntry>, ApiError> {
    // The link is spent by accepting, so find the event first.
    let invite = state.store.partner_invite(&ptoken).await?;
    let mine = state
        .store
        .accept_partner(&ptoken, &user.user.id, now())
        .await?;
    if let Some(invite) = invite {
        changed_by_link(&state, &invite.join_token).await?;
    }
    Ok(Json(mine))
}

async fn decline(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(ptoken): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.decline_partner(&ptoken, &user.user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<SignupList>, ApiError> {
    let list = state
        .store
        .signup_list(&user.user.id, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(list))
}

/// The organiser's fresh list after a change, with the streams told.
async fn organiser_changed(
    state: &AppState,
    owner: &str,
    id: &str,
) -> Result<Json<SignupList>, ApiError> {
    changed(state, id, owner);
    list_of(state, owner, id).await
}

async fn list_of(state: &AppState, owner: &str, id: &str) -> Result<Json<SignupList>, ApiError> {
    let list = state
        .store
        .signup_list(owner, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(list))
}

async fn remove(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, entry_id)): ApiPath<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .remove_signup(&user.user.id, &id, &entry_id, now())
        .await?;
    changed(&state, &id, &user.user.id);
    Ok(StatusCode::NO_CONTENT)
}

async fn split(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath((id, entry_id)): ApiPath<(String, String)>,
) -> Result<Json<SignupList>, ApiError> {
    state
        .store
        .split_signup(&user.user.id, &id, &entry_id, now())
        .await?;
    organiser_changed(&state, &user.user.id, &id).await
}

async fn pair(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<PairBody>,
) -> Result<Json<SignupList>, ApiError> {
    let [a, b] = &body.entries;
    state
        .store
        .pair_signups(&user.user.id, &id, a, b, now())
        .await?;
    organiser_changed(&state, &user.user.id, &id).await
}
