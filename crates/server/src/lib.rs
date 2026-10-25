pub mod accounts_store;
pub mod api;
pub mod auth;
pub mod client_ip;
pub mod commands;
pub mod config;
pub mod db;
pub mod hub;
pub mod ratelimit;
pub mod retention;
pub mod store;
pub mod tokens;
pub mod web;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub use config::Config;
pub use hub::Hub;
pub use store::Store;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Store,
    pub hub: Arc<Hub>,
    /// Cancelled when the server shuts down; every SSE stream ends on it.
    pub shutdown: CancellationToken,
    /// Bounds concurrent argon2 work to [`auth::HASH_PERMITS`]; see [`auth::hash_password`].
    pub password_permits: Arc<Semaphore>,
    /// The named rate limiters (`state.limits.login_ip`, ...); see [`ratelimit::Limits`].
    pub limits: Arc<ratelimit::Limits>,
}

impl AppState {
    pub fn new(config: Arc<Config>, store: Store) -> AppState {
        AppState {
            config,
            store,
            hub: Arc::new(Hub::new()),
            shutdown: CancellationToken::new(),
            password_permits: Arc::new(Semaphore::new(auth::HASH_PERMITS)),
            limits: Arc::new(ratelimit::Limits::new()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .merge(api::routes())
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/", axum::routing::any(api_not_found))
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        // After every route: it applies to the method routers registered so far.
        .method_not_allowed_fallback(api::method_not_allowed)
        .fallback(web::static_handler)
        .layer(DefaultBodyLimit::max(api::MAX_BODY_BYTES))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "version": state.config.version,
        "commit": state.config.commit,
    }))
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" })))
}
