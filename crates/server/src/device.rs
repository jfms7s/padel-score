//! The device-code sign-in flow (RFC 8628 shapes) and device management.
//!
//! The watch:
//!
//! - `POST /api/device/start` (public) takes `{device_name}`, trimmed and cut to
//!   [`MAX_DEVICE_NAME_CHARS`] chars, [`DEFAULT_DEVICE_NAME`] when missing or blank. It is 200
//!   `{device_code, user_code: "ABCD-EFGH", verification_uri: "/link", expires_in: 600,
//!   interval: 5}`. Limited by `device_start`, per client address.
//! - `POST /api/device/token` (public) takes `{device_code}`. While nothing is issued it is 400
//!   `{"error": "authorization_pending" | "slow_down" | "expired_token" | "access_denied"}`;
//!   an unknown, purged or already used code is `expired_token`, never 404. Once approved it is
//!   200 `{access_token, token_type: "bearer"}`, exactly once: the poll mints the token in the
//!   transaction that marks the code consumed ([`crate::Store::poll_device`]).
//!
//! The person approving, on a session only (a device token is 403):
//!
//! - `GET /api/device/{user_code}` is `{device_name, created_secs_ago, expires_in_secs}` for a
//!   pending code. Unknown, expired, decided and consumed codes are all 404 `not_found`. The
//!   typed code is normalized (case, spaces, the dash).
//! - `POST /api/device/{user_code}/approve` and `/deny` are 204. A decided code is 409
//!   `device_code_used`; unknown or expired, 404. Both count every call against the user's
//!   `device_approve` budget.
//! - `GET /api/me/devices` is the caller's signed-in devices,
//!   `[{id, name, prefix, created_at, last_used_at?}]`, never a token.
//!   `DELETE /api/me/devices/{id}` revokes one of the caller's own: 204; anyone else's, an
//!   unknown or an already revoked one is 404. A revoked device is 401 on its next request, and
//!   its open streams end on their next credential re-check.
//!
//! The user code is only ever read from the path, never from a query string: the page
//! (`/link`) never pre-fills it from its URL, so a phishing link cannot hand someone a code to
//! approve. Every response here carries `Referrer-Policy: no-referrer` and
//! `Cache-Control: no-store`.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::accounts::secret_headers;
use crate::accounts_store::{
    DEVICE_CODE_TTL_SECS, DEVICE_POLL_INTERVAL_SECS, DeviceToken, PollOutcome,
};
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::client_ip::{ClientIp, rate_key};
use crate::store::StoreError;

/// The page that shows the code's entry form, relative to the server.
pub const VERIFICATION_URI: &str = "/link";
/// The name a device gets when it sends none.
pub const DEFAULT_DEVICE_NAME: &str = "Watch";
/// The longest device name kept, in chars, once trimmed.
pub const MAX_DEVICE_NAME_CHARS: usize = 40;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/device/start", post(start))
        .route("/api/device/token", post(token))
        .route("/api/device/{user_code}", get(lookup))
        .route("/api/device/{user_code}/approve", post(approve))
        .route("/api/device/{user_code}/deny", post(deny))
        .route("/api/me/devices", get(list_devices))
        .route("/api/me/devices/{id}", delete(revoke_device))
        .layer(axum::middleware::map_response(secret_headers))
}

/// The stored name for a requested one: trimmed, cut to [`MAX_DEVICE_NAME_CHARS`], and
/// [`DEFAULT_DEVICE_NAME`] when nothing is left.
pub fn device_name(requested: Option<&str>) -> String {
    let name: String = requested
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_DEVICE_NAME_CHARS)
        .collect();
    // Cutting may leave trailing whitespace.
    let name = name.trim_end();
    if name.is_empty() {
        DEFAULT_DEVICE_NAME.to_owned()
    } else {
        name.to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct StartRequest {
    #[serde(default)]
    device_name: Option<String>,
}

#[derive(Serialize)]
struct StartResponse {
    device_code: String,
    user_code: String,
    verification_uri: &'static str,
    expires_in: i64,
    interval: i64,
}

async fn start(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(req): ApiJson<StartRequest>,
) -> Result<Json<StartResponse>, ApiError> {
    let t = now();
    state
        .limits
        .device_start
        .try_acquire(&rate_key(ip), t)
        .map_err(|retry_after| ApiError::TooMany { retry_after })?;
    let name = device_name(req.device_name.as_deref());
    let started = state.store.start_device(&name, t).await?;
    Ok(Json(StartResponse {
        device_code: started.device_code,
        user_code: started.user_code,
        verification_uri: VERIFICATION_URI,
        expires_in: DEVICE_CODE_TTL_SECS,
        interval: DEVICE_POLL_INTERVAL_SECS,
    }))
}

#[derive(Deserialize)]
struct TokenRequest {
    device_code: String,
}

/// An RFC 8628 §3.5 error: 400 with the code.
fn poll_error(code: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code }))).into_response()
}

async fn token(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<TokenRequest>,
) -> Result<Response, ApiError> {
    let outcome = match state.store.poll_device(&req.device_code, now()).await {
        Ok(outcome) => outcome,
        // Unknown or purged; or a poll that lost the race to consume the code.
        Err(StoreError::NotFound | StoreError::Conflict(_)) => PollOutcome::Expired,
        Err(e) => return Err(e.into()),
    };
    Ok(match outcome {
        PollOutcome::Issued { raw_token } => Json(json!({
            "access_token": raw_token,
            "token_type": "bearer",
        }))
        .into_response(),
        PollOutcome::Pending => poll_error("authorization_pending"),
        PollOutcome::SlowDown => poll_error("slow_down"),
        PollOutcome::Denied => poll_error("access_denied"),
        PollOutcome::Expired | PollOutcome::Consumed => poll_error("expired_token"),
    })
}

#[derive(Serialize)]
struct PendingDevice {
    device_name: String,
    created_secs_ago: i64,
    expires_in_secs: i64,
}

async fn lookup(
    State(state): State<AppState>,
    SessionUser(_): SessionUser,
    ApiPath(user_code): ApiPath<String>,
) -> Result<Json<PendingDevice>, ApiError> {
    let t = now();
    let code = state
        .store
        .device_by_user_code(&user_code)
        .await?
        .filter(|code| code.is_pending(t) && code.consumed_at.is_none())
        .ok_or(ApiError::NotFound)?;
    Ok(Json(PendingDevice {
        device_name: code.device_name,
        created_secs_ago: (t - code.created_at).max(0),
        expires_in_secs: code.expires_at - t,
    }))
}

/// Count one decision by `user_id` against the `device_approve` budget.
fn limit_decision(state: &AppState, user_id: &str, t: i64) -> Result<(), ApiError> {
    state
        .limits
        .device_approve
        .try_acquire(user_id, t)
        .map_err(|retry_after| ApiError::TooMany { retry_after })
}

async fn approve(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(user_code): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    let t = now();
    limit_decision(&state, &user.user.id, t)?;
    state
        .store
        .approve_device(&user_code, &user.user.id, t)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn deny(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(user_code): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    let t = now();
    limit_decision(&state, &user.user.id, t)?;
    state.store.deny_device(&user_code, t).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct DeviceBody {
    id: String,
    name: String,
    prefix: String,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_used_at: Option<i64>,
}

impl From<DeviceToken> for DeviceBody {
    fn from(d: DeviceToken) -> DeviceBody {
        DeviceBody {
            id: d.id,
            name: d.name,
            prefix: d.prefix,
            created_at: d.created_at,
            last_used_at: d.last_used_at,
        }
    }
}

async fn list_devices(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<Vec<DeviceBody>>, ApiError> {
    let devices = state.store.list_devices(&user.user.id).await?;
    Ok(Json(devices.into_iter().map(DeviceBody::from).collect()))
}

async fn revoke_device(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state.store.revoke_device(&user.user.id, &id, now()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_rules() {
        assert_eq!(device_name(None), "Watch");
        assert_eq!(device_name(Some(" \t ")), "Watch");
        assert_eq!(device_name(Some("  SM-R960 ")), "SM-R960");
        let long = "é".repeat(50);
        assert_eq!(device_name(Some(&long)).chars().count(), 40);
        let cut_at_space = format!("{}  tail", "a".repeat(39));
        assert_eq!(device_name(Some(&cut_at_space)), "a".repeat(39));
    }
}
