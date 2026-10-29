//! `GET /api/me/organised` for a signed-in session: the caller's [`Organised`]. The response
//! carries join and board links, so it is `Cache-Control: no-store` and
//! `Referrer-Policy: no-referrer` ([`accounts::secret_headers`]).

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, now};
use crate::auth::SessionUser;
use crate::organised::Organised;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/organised", get(organised))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

async fn organised(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
) -> Result<Json<Organised>, ApiError> {
    Ok(Json(state.store.organised(&user.user.id, now()).await?))
}
