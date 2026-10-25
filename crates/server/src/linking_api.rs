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
//! Lookups and accepts count against the caller's `claims` budget.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::accounts::{self, fragment_url};
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::linking::ClaimPreview;

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
