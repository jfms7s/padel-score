//! The push routes, all `Cache-Control: no-store` ([`accounts::secret_headers`]):
//!
//! - `GET /api/push/key` is `{ "key" }`, the VAPID public key (`applicationServerKey`); no
//!   account needed.
//! - `GET /api/me/push` is `{ "devices": n }`, the caller's subscribed devices.
//! - `POST /api/me/push` with a browser `PushSubscription` is 204. An endpoint that is not an
//!   `https` public host, or keys of the wrong size, is `422 validation` on `endpoint`.
//! - `DELETE /api/me/push` `{endpoint}` is 204, also when it is not subscribed.
//! - `POST /api/me/push/test` is 202: it queues a test notice to the caller.
//!
//! The `/api/me/*` routes take a signed-in session; a device token is 403.

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, now};
use crate::auth::SessionUser;
use crate::push::{self, Notice, NoticeKind, Payload, PushSubscription};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/push/key", get(key))
        .route(
            "/api/me/push",
            get(devices).post(subscribe).delete(unsubscribe),
        )
        .route("/api/me/push/test", post(test))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

#[derive(Debug, Serialize)]
struct KeyBody {
    key: String,
}

#[derive(Debug, Serialize)]
struct DevicesBody {
    devices: usize,
}

#[derive(Debug, Deserialize)]
struct Unsubscribe {
    endpoint: String,
}

async fn key(State(state): State<AppState>) -> Result<Json<KeyBody>, ApiError> {
    let keys = push::vapid_keys(&state.store).await?;
    Ok(Json(KeyBody {
        key: keys.public_b64(),
    }))
}

async fn devices(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<DevicesBody>, ApiError> {
    let devices = state.store.push_device_count(&user.user.id).await?;
    Ok(Json(DevicesBody { devices }))
}

async fn subscribe(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(sub): ApiJson<PushSubscription>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .save_push_subscription(&user.user.id, &sub, now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unsubscribe(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiJson(req): ApiJson<Unsubscribe>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .delete_push_subscription(&user.user.id, &req.endpoint)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<StatusCode, ApiError> {
    let notice = Notice {
        user_id: user.user.id,
        payload: Payload {
            kind: NoticeKind::Test,
            url: "/account".to_owned(),
            ..Payload::default()
        },
        dedupe: None,
    };
    state.store.queue(&[notice], now()).await?;
    state.push.wake();
    Ok(StatusCode::ACCEPTED)
}
