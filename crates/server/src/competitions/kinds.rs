//! The seam between the shared event code and the two kinds. Every function matches on [`Kind`]
//! and calls the same-named function in `tournament.rs` (phase 7) or `non_stop.rs` (phase 8),
//! so neither phase edits the other's file, and this one only when the seam itself changes.

use sqlx::SqliteConnection;

use super::places::Place;
use super::{Audience, Competition, EventTeam, Kind, non_stop, tournament};
use crate::store::{StoreError, Tx};

/// The kind's settings for storage: `raw` (or the kind's defaults when `None`) checked and
/// filled in.
pub fn normalize_settings(
    kind: Kind,
    raw: Option<&serde_json::Value>,
) -> Result<serde_json::Value, StoreError> {
    match kind {
        Kind::Tournament => tournament::normalize_settings(raw),
        Kind::NonStop => non_stop::normalize_settings(raw),
    }
}

/// Whether the event may start with these teams.
pub async fn check_start(
    kind: Kind,
    conn: &mut SqliteConnection,
    ev: &Competition,
    teams: &[EventTeam],
) -> Result<(), StoreError> {
    match kind {
        Kind::Tournament => tournament::check_start(conn, ev, teams).await,
        Kind::NonStop => non_stop::check_start(conn, ev, teams).await,
    }
}

/// Runs in the transaction that moves the event to `running`.
pub async fn on_start(
    kind: Kind,
    tx: &mut Tx<'_>,
    ev: &Competition,
    now: i64,
) -> Result<(), StoreError> {
    match kind {
        Kind::Tournament => tournament::on_start(tx, ev, now).await,
        Kind::NonStop => non_stop::on_start(tx, ev, now).await,
    }
}

/// Runs after every applied command on an event match (points, undos that un-finish it,
/// typed results), inside its transaction; read `match_results` to know whether it is finished.
pub async fn on_result(
    kind: Kind,
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    match kind {
        Kind::Tournament => tournament::on_result(tx, ev, match_id, now).await,
        Kind::NonStop => non_stop::on_result(tx, ev, match_id, now).await,
    }
}

/// Whether the kind works out the final places itself, so the coordinator's Finish screen
/// sends none. Must agree with [`computed_places`] returning `Some` once the event can finish.
pub fn computes_places(kind: Kind) -> bool {
    match kind {
        Kind::Tournament => tournament::computes_places(),
        Kind::NonStop => non_stop::computes_places(),
    }
}

/// The places the kind works out itself, or `None` when the coordinator supplies them.
pub async fn computed_places(
    kind: Kind,
    conn: &mut SqliteConnection,
    ev: &Competition,
) -> Result<Option<Vec<Place>>, StoreError> {
    match kind {
        Kind::Tournament => tournament::computed_places(conn, ev).await,
        Kind::NonStop => non_stop::computed_places(conn, ev).await,
    }
}

/// The kind's part of the event view for `audience`, if it has one.
pub async fn kind_data(
    kind: Kind,
    conn: &mut SqliteConnection,
    ev: &Competition,
    audience: Audience,
) -> Result<Option<serde_json::Value>, StoreError> {
    match kind {
        Kind::Tournament => tournament::kind_data(conn, ev, audience).await,
        Kind::NonStop => non_stop::kind_data(conn, ev, audience).await,
    }
}
