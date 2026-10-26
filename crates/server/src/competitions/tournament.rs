//! The tournament kind. Owned by phase 7: phase 8 never edits this file. The phase 6 stubs
//! below do nothing; phase 7 fills them in.

pub mod bracket;
pub mod store;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use super::places::{Place, write_placings_in};
use super::{Audience, Competition, EventTeam, Status, teams_in};
use crate::store::{StoreError, Tx};

use bracket::{Layout, Plan};

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

/// A bracket match's result changed (a point, an undo or a typed result): move its winner and
/// loser on. While the event runs, the bracket's plan is carried out as it is. After Finish no
/// match can be made or taken off a court, so a plan that would create or withdraw one becomes
/// a hold on the match instead (decision 8); any other plan is carried out and, if the bracket
/// is then complete, the placings are written again with the event's `finished_at`.
///
/// Never refuses: it runs inside every command on an event match, and a state the bracket can't
/// act on is held, not an error. A match outside the bracket (made by hand) is ignored.
pub async fn on_result(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    let Some(bracket) = store::load_in(tx, ev).await? else {
        return Ok(());
    };
    let Some(pos) = bracket.node_by_match(match_id).map(|n| n.pos) else {
        return Ok(());
    };
    let plan = bracket.on_result(pos);
    match ev.status {
        Status::Running => {
            store::apply_plan_in(tx, ev, &bracket, &plan, now).await?;
        }
        Status::Finished => {
            let plan = if plan.create.is_empty() && plan.withdraw.is_empty() {
                plan
            } else {
                Plan {
                    held: vec![(pos, true)],
                    ..Plan::default()
                }
            };
            store::apply_plan_in(tx, ev, &bracket, &plan, now).await?;
            let places = store::load_in(tx, ev).await?.and_then(|b| b.places());
            if let Some(places) = places {
                let teams = teams_in(tx, &ev.id).await?;
                write_placings_in(tx, ev, &teams, &places, ev.finished_at.unwrap_or(now)).await?;
            }
        }
        // A bracket is drawn at Start, so an event in setup has none.
        Status::Setup => {}
    }
    Ok(())
}

/// A tournament always works out its final places from the bracket.
pub fn computes_places() -> bool {
    true
}

/// The bracket's final places: `Conflict("bracket_needs_decision")` while a match is held,
/// `Conflict("bracket_unfinished")` until every match has a winner (or with no bracket).
pub async fn computed_places(
    conn: &mut SqliteConnection,
    ev: &Competition,
) -> Result<Option<Vec<Place>>, StoreError> {
    let bracket = store::load_in(conn, ev)
        .await?
        .ok_or(StoreError::Conflict("bracket_unfinished"))?;
    if bracket.nodes.iter().any(|n| n.held) {
        return Err(StoreError::Conflict("bracket_needs_decision"));
    }
    bracket
        .places()
        .map(Some)
        .ok_or(StoreError::Conflict("bracket_unfinished"))
}

pub async fn kind_data(
    _conn: &mut SqliteConnection,
    _ev: &Competition,
    _audience: Audience,
) -> Result<Option<serde_json::Value>, StoreError> {
    Ok(None)
}
