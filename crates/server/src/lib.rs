pub mod accounts;
pub mod accounts_store;
pub mod api;
pub mod auth;
pub mod client_ip;
pub mod commands;
pub mod config;
pub mod db;
pub mod device;
pub mod hub;
pub mod linking;
pub mod owned;
pub mod players;
pub mod players_api;
pub mod ratelimit;
pub mod results;
pub mod retention;
pub mod store;
pub mod tokens;
pub mod web;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, MatchedPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use serde_json::json;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{MakeSpan, TraceLayer};

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
    /// The one-time setup code while the users table is empty and `SETUP_TOKEN` is unset; set by
    /// [`accounts::init_setup_code`] at startup and cleared by a successful setup.
    pub setup_code: Arc<Mutex<Option<String>>>,
    /// How often a signed-in SSE stream re-checks its credential, ending once it no longer
    /// holds (a revoked device, a deleted session, a disabled user). [`CREDENTIAL_RECHECK`] in
    /// production; tests shorten it.
    pub credential_recheck: Duration,
    /// The open list streams per user, capped at [`owned::MAX_LIST_STREAMS`].
    pub list_streams: Arc<owned::ListStreams>,
}

/// The default [`AppState::credential_recheck`].
pub const CREDENTIAL_RECHECK: Duration = Duration::from_secs(30);

impl AppState {
    pub fn new(config: Arc<Config>, store: Store) -> AppState {
        AppState {
            config,
            store,
            hub: Arc::new(Hub::new()),
            shutdown: CancellationToken::new(),
            password_permits: Arc::new(Semaphore::new(auth::HASH_PERMITS)),
            limits: Arc::new(ratelimit::Limits::new()),
            setup_code: Arc::new(Mutex::new(None)),
            credential_recheck: CREDENTIAL_RECHECK,
            list_streams: Arc::new(owned::ListStreams::new()),
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

/// The request trace layer `main.rs` serves with: tower-http's HTTP defaults, except that each
/// request's span names its matched route template (`/api/invites/{id}`), never the URI. Paths
/// can carry secrets (invite codes, phase 2 scorer tokens) and a query string anything, so
/// neither may reach a log. A request no route matched (the static web files, and the PWA's
/// pages such as `/s/{scorer_token}`) logs the fixed label [`STATIC_ROUTE`].
///
/// Add it with [`Router::layer`] on the router from [`router`]: only there do requests already
/// carry their [`MatchedPath`].
pub fn trace_layer() -> TraceLayer<SharedClassifier<ServerErrorsAsFailures>, RouteSpan> {
    TraceLayer::new_for_http().make_span_with(RouteSpan)
}

/// The route [`trace_layer`] logs for a request no route matched.
pub const STATIC_ROUTE: &str = "<static>";

/// The [`MakeSpan`] of [`trace_layer`]: a `request` span at debug level with the method, the
/// matched route and the HTTP version.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteSpan;

impl<B> MakeSpan<B> for RouteSpan {
    fn make_span(&mut self, request: &axum::http::Request<B>) -> tracing::Span {
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map_or(STATIC_ROUTE, MatchedPath::as_str);
        tracing::debug_span!(
            "request",
            method = %request.method(),
            route,
            version = ?request.version(),
        )
    }
}
