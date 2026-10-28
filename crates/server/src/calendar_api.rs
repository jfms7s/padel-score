//! `GET /api/me/calendar?from=<secs>` for a signed-in session: the caller's [`Calendar`].
//!
//! `from` is required (a missing or non-integer value is `400 bad_request`) and is raised to at
//! most two days before now, so the calendar never reads further back. The response carries
//! board, join and partner links, so it is `Cache-Control: no-store` and
//! `Referrer-Policy: no-referrer` ([`accounts::secret_headers`]).

use axum::Router;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Json;
use axum::routing::get;
use serde::Deserialize;

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, now};
use crate::auth::SessionUser;
use crate::calendar::Calendar;

/// How far back `from` may reach.
const LOOK_BACK_SECS: i64 = 2 * crate::calendar::DAY_SECS;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/me/calendar", get(calendar))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

#[derive(Debug, Deserialize)]
struct CalendarQuery {
    from: Option<String>,
}

async fn calendar(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    query: Result<Query<CalendarQuery>, QueryRejection>,
) -> Result<Json<Calendar>, ApiError> {
    let Ok(Query(query)) = query else {
        return Err(ApiError::BadRequest);
    };
    let from: i64 = query
        .from
        .as_deref()
        .and_then(|text| text.parse().ok())
        .ok_or(ApiError::BadRequest)?;
    let at = now();
    let from = from.max(at.saturating_sub(LOOK_BACK_SECS));
    Ok(Json(state.store.calendar(&user.user.id, from, at).await?))
}
