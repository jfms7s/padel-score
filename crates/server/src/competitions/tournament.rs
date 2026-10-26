//! The tournament kind. Owned by phase 7: phase 8 never edits this file. The phase 6 stubs
//! below do nothing; phase 7 fills them in.

pub(crate) mod bracket;
pub mod store;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use super::places::Place;
use super::{Audience, Competition, EventTeam};
use crate::store::{StoreError, Tx};

use bracket::Layout;

/// A tournament's settings: exactly `{"full_placement": bool}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Whether every place is played for, not only the top four.
    #[serde(default)]
    pub full_placement: bool,
}

/// The event's settings; stored settings were normalized, so a bad value falls back to the
/// defaults rather than failing a read.
pub fn settings_of(ev: &Competition) -> Settings {
    serde_json::from_value(ev.settings.clone()).unwrap_or_default()
}

/// Absent or `{}` gives `{"full_placement": false}`; any other key or type is
/// `InvalidField("settings")`.
pub fn normalize_settings(
    raw: Option<&serde_json::Value>,
) -> Result<serde_json::Value, StoreError> {
    let settings = match raw {
        None => Settings::default(),
        // A struct would also read an array of values; settings are an object.
        Some(v) if v.is_object() => {
            serde_json::from_value(v.clone()).map_err(|_| StoreError::InvalidField("settings"))?
        }
        Some(_) => return Err(StoreError::InvalidField("settings")),
    };
    serde_json::to_value(settings).map_err(|_| StoreError::InvalidField("settings"))
}

/// A tournament starts with 4, 8, 16 or 32 teams (`Conflict("team_count")`).
pub async fn check_start(
    _conn: &mut SqliteConnection,
    ev: &Competition,
    teams: &[EventTeam],
) -> Result<(), StoreError> {
    match Layout::new(teams.len(), settings_of(ev).full_placement) {
        Some(_) => Ok(()),
        None => Err(StoreError::Conflict("team_count")),
    }
}

/// Draw the bracket at random and put the first matches on the free courts.
pub async fn on_start(tx: &mut Tx<'_>, ev: &Competition, now: i64) -> Result<(), StoreError> {
    store::draw_in(tx, ev, now).await
}

pub async fn on_result(
    _tx: &mut Tx<'_>,
    _ev: &Competition,
    _match_id: &str,
    _now: i64,
) -> Result<(), StoreError> {
    Ok(())
}

/// Whether this kind works out the final places itself (`computed_places` returns `Some`).
pub fn computes_places() -> bool {
    false
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
