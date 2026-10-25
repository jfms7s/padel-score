//! Link-code and link routes, for a signed-in session user (a device token is 403). Every
//! response carries `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]); a code travels in request bodies and the `/link-player#`
//! fragment only, never in a path or query.
//!
//! - `POST /api/players/{id}/link-code` is 201 `{url, expires_at}`, 404 for an unknown or
//!   another user's entry, 409 `player_linked`.
//! - `POST /api/link-codes/lookup` `{code}` is 200 with the `ClaimPreview`, or 410
//!   `link_code_invalid` (one answer for an unknown, expired, used or no longer linkable code).
//! - `POST /api/link-codes/accept` `{code}` is 200 `{player_id}`, 410 `link_code_invalid`, or
//!   409 `player_linked` / `link_overlaps`.
//! - `POST /api/players/{id}/link-me` and `DELETE /api/players/{id}/link` are 204, or 404.
//!
//! Claims (see [`crate::claims`]), all for a session user:
//!
//! - `GET /api/view/{token}/slots` is `{items: [SlotState]}`, or 404 for an unknown token.
//! - `POST /api/claims/preview` `ClaimTarget` is the `ClaimPreview`, or 404 `not_found`.
//! - `POST /api/claims` `ClaimTarget` is 201 `MyClaim` (a new or re-opened claim), 200 (a
//!   pending one that already existed, or the coordinator's own, linked at once), 404, or 409
//!   `already_yours`, `player_linked`, `claim_declined`, `too_many_claims`.
//! - `GET /api/claims` is the coordinator's inbox `{items: [Claim]}`;
//!   `POST /api/claims/{id}/approve` and `/decline` are 204, 404, or 409 `claim_decided` (approve
//!   also `player_linked`, `link_overlaps`).
//! - `GET /api/me/claims` is `{items: [MyClaim]}`; `DELETE /api/me/claims/{id}` is 204, 404 or
//!   409 `claim_decided`.
//! - `GET /api/me/links` is `{items: [MyLink]}`; `DELETE /api/me/links/{player_id}` is 204 or 404.
//!
//! Lookups, accepts, claim previews and claim creates count against the caller's `claims` budget.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AppState;
use crate::accounts::{self, fragment_url};
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::claims::{Claim, ClaimTarget, MyClaim, SlotState};
use crate::linking::{ClaimPreview, MyLink};

/// Where the PWA's link page lives; the code follows in the fragment.
const LINK_PAGE: &str = "/link-player";

const LINK_CODE_INVALID: ApiError = ApiError::Gone("link_code_invalid");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/players/{id}/link-code", post(create_code))
        .route("/api/players/{id}/link-me", post(link_me))
        .route("/api/players/{id}/link", delete(unlink))
        .route("/api/link-codes/lookup", post(lookup))
        .route("/api/link-codes/accept", post(accept))
        .route("/api/view/{token}/slots", get(slots))
        .route("/api/claims/preview", post(claim_preview))
        .route("/api/claims", get(incoming).post(create_claim))
        .route("/api/claims/{id}/approve", post(approve))
        .route("/api/claims/{id}/decline", post(decline))
        .route("/api/me/claims", get(my_claims))
        .route("/api/me/claims/{id}", delete(withdraw))
        .route("/api/me/links", get(my_links))
        .route("/api/me/links/{player_id}", delete(unlink_self))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// Count one claim-or-code call by `user_id` against the `claims` budget.
pub(crate) fn limit_claims(state: &AppState, user_id: &str, t: i64) -> Result<(), ApiError> {
    state
        .limits
        .claims
        .try_acquire(user_id, t)
        .map_err(|retry_after| ApiError::TooMany { retry_after })
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

async fn create_code(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (code, expires_at) = state
        .store
        .create_link_code(&user.user.id, &id, now())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "url": fragment_url(LINK_PAGE, &code), "expires_at": expires_at })),
    ))
}

async fn lookup(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(body): ApiJson<CodeBody>,
) -> Result<Json<ClaimPreview>, ApiError> {
    let t = now();
    limit_claims(&state, &user.user.id, t)?;
    state
        .store
        .link_code_preview(&body.code, t)
        .await?
        .map(Json)
        .ok_or(LINK_CODE_INVALID)
}

async fn accept(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(body): ApiJson<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    let t = now();
    limit_claims(&state, &user.user.id, t)?;
    let player_id = state
        .store
        .accept_link_code(&body.code, &user.user.id, t)
        .await?
        .ok_or(LINK_CODE_INVALID)?;
    Ok(Json(json!({ "player_id": player_id })))
}

async fn link_me(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.link_me(&user.user.id, &id, now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlink(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.unlink(&user.user.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Items<T> {
    items: Vec<T>,
}

async fn slots(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(token): ApiPath<String>,
) -> Result<Json<Items<SlotState>>, ApiError> {
    let items = state
        .store
        .slots(&token, &user.user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(Items { items }))
}

async fn claim_preview(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(target): ApiJson<ClaimTarget>,
) -> Result<Json<ClaimPreview>, ApiError> {
    limit_claims(&state, &user.user.id, now())?;
    state
        .store
        .claim_preview(&target)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn create_claim(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(target): ApiJson<ClaimTarget>,
) -> Result<(StatusCode, Json<MyClaim>), ApiError> {
    let t = now();
    limit_claims(&state, &user.user.id, t)?;
    let (claim, created) = state.store.create_claim(&target, &user.user.id, t).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(claim)))
}

async fn incoming(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<Items<Claim>>, ApiError> {
    let items = state.store.incoming_claims(&user.user.id).await?;
    Ok(Json(Items { items }))
}

async fn approve(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.approve_claim(&user.user.id, &id, now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn decline(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.decline_claim(&user.user.id, &id, now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn my_claims(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<Items<MyClaim>>, ApiError> {
    let items = state.store.my_claims(&user.user.id).await?;
    Ok(Json(Items { items }))
}

async fn withdraw(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .withdraw_claim(&user.user.id, &id, now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn my_links(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<Items<MyLink>>, ApiError> {
    let items = state.store.my_links(&user.user.id).await?;
    Ok(Json(Items { items }))
}

async fn unlink_self(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(player_id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.unlink_self(&user.user.id, &player_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
