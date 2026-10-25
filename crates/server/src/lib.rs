pub mod config;
pub mod db;
pub mod store;
pub mod tokens;
pub mod web;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use serde_json::json;

pub use config::Config;
pub use store::Store;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Store,
}

impl AppState {
    pub fn new(config: Arc<Config>, store: Store) -> AppState {
        AppState { config, store }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api", axum::routing::any(api_not_found))
        .route("/api/", axum::routing::any(api_not_found))
        .route("/api/{*rest}", axum::routing::any(api_not_found))
        .fallback(web::static_handler)
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
