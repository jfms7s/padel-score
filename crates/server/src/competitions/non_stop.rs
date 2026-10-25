//! The non-stop kind. Owned by phase 8: phase 7 never edits this file. The phase 6 stubs
//! below do nothing; phase 8 fills them in.

use sqlx::SqliteConnection;

use super::places::Place;
use super::{Audience, Competition, EventTeam};
use crate::store::{StoreError, Tx};

pub fn normalize_settings(
    raw: Option<&serde_json::Value>,
) -> Result<serde_json::Value, StoreError> {
    super::empty_settings(raw)
}

pub async fn check_start(
    _conn: &mut SqliteConnection,
    _ev: &Competition,
    _teams: &[EventTeam],
) -> Result<(), StoreError> {
    Ok(())
}

pub async fn on_start(_tx: &mut Tx<'_>, _ev: &Competition, _now: i64) -> Result<(), StoreError> {
    Ok(())
}

pub async fn on_result(
    _tx: &mut Tx<'_>,
    _ev: &Competition,
    _match_id: &str,
    _now: i64,
) -> Result<(), StoreError> {
    Ok(())
}

pub async fn computed_places(
    _conn: &mut SqliteConnection,
    _ev: &Competition,
) -> Result<Option<Vec<Place>>, StoreError> {
    Ok(None)
}

pub async fn kind_data(
    _conn: &mut SqliteConnection,
    _ev: &Competition,
    _audience: Audience,
) -> Result<Option<serde_json::Value>, StoreError> {
    Ok(None)
}
