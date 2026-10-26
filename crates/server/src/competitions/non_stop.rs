//! The non-stop kind. Owned by phase 8: phase 7 never edits this file. Every match is made at
//! start (`schedule`), the clock runs one slot at a time (`clock`), and the standings are
//! worked out on read from `match_results` (`standings`).

pub mod clock;
mod schedule;
pub mod standings;

use padel_scoring::Team;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Row, SqliteConnection};

use super::courts::{courts_in, create_event_match_in, next_queued_in};
use super::places::{Place, write_placings_in};
use super::{Audience, Competition, EventTeam, Status, teams_in};
use crate::results::FinalScore;
use crate::store::{StoreError, Tx};
use clock::{current_clock, slot_of_match_in, slots_in, system_now_ms};
use schedule::{plan_slots, round_robin};
use standings::{Outcome, StandingRow, places_of};

pub const DEFAULT_MATCH_MINUTES: u32 = 15;
pub const MAX_MATCH_MINUTES: u32 = 60;
pub const MAX_LEGS: u32 = 3;
pub const MAX_SCHEDULE_MATCHES: usize = 240;

/// How the teams are laid out in the schedule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// The event's team list order.
    #[default]
    Listed,
    /// Shuffled once, at start.
    Random,
}

/// A non-stop's `competitions.settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "default_match_minutes")]
    pub match_minutes: u32,
    #[serde(default = "default_legs")]
    pub legs: u32,
    #[serde(default)]
    pub order: Order,
}

fn default_match_minutes() -> u32 {
    DEFAULT_MATCH_MINUTES
}

fn default_legs() -> u32 {
    1
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            match_minutes: DEFAULT_MATCH_MINUTES,
            legs: 1,
            order: Order::Listed,
        }
    }
}

/// The settings of a stored non-stop event (always valid; a decode failure is `Corrupt`).
pub fn settings_of(ev: &Competition) -> Result<Settings, StoreError> {
    serde_json::from_value(ev.settings.clone())
        .map_err(|e| StoreError::Corrupt(format!("non-stop settings: {e}")))
}

pub fn normalize_settings(
    raw: Option<&serde_json::Value>,
) -> Result<serde_json::Value, StoreError> {
    let settings = match raw {
        None => Settings::default(),
        Some(v) if !v.is_object() => return Err(StoreError::InvalidField("settings")),
        Some(v) => {
            let s: Settings = serde_json::from_value(v.clone())
                .map_err(|_| StoreError::InvalidField("settings"))?;
            if !(1..=MAX_MATCH_MINUTES).contains(&s.match_minutes)
                || !(1..=MAX_LEGS).contains(&s.legs)
            {
                return Err(StoreError::InvalidField("settings"));
            }
            s
        }
    };
    serde_json::to_value(settings).map_err(|e| StoreError::Corrupt(e.to_string()))
}

/// Matches in the whole schedule: `legs x n(n-1)/2`.
fn schedule_size(teams: usize, legs: u32) -> usize {
    teams
        .saturating_mul(teams.saturating_sub(1))
        .saturating_div(2)
        .saturating_mul(legs as usize)
}

pub async fn check_start(
    _conn: &mut SqliteConnection,
    ev: &Competition,
    teams: &[EventTeam],
) -> Result<(), StoreError> {
    if !teams.len().is_multiple_of(2) {
        return Err(StoreError::Conflict("odd_teams"));
    }
    if schedule_size(teams.len(), settings_of(ev)?.legs) > MAX_SCHEDULE_MATCHES {
        return Err(StoreError::Conflict("schedule_too_big"));
    }
    Ok(())
}

/// Fisher-Yates, with `next` as the random source.
pub(crate) fn shuffle(order: &mut [usize], mut next: impl FnMut() -> u64) {
    for i in (1..order.len()).rev() {
        let bound = u64::try_from(i + 1).unwrap_or(u64::MAX);
        let j = usize::try_from(next() % bound).unwrap_or(0);
        order.swap(i, j);
    }
}

/// Creates the whole schedule: every slot, and every match queued on its court in play order.
pub async fn on_start(tx: &mut Tx<'_>, ev: &Competition, now: i64) -> Result<(), StoreError> {
    let settings = settings_of(ev)?;
    let teams = teams_in(tx, &ev.id).await?;
    let courts = courts_in(tx, &ev.id).await?;
    let mut order: Vec<usize> = (0..teams.len()).collect();
    if settings.order == Order::Random {
        let mut failed = false;
        shuffle(&mut order, || {
            getrandom::u64().unwrap_or_else(|_| {
                failed = true;
                0
            })
        });
        if failed {
            return Err(StoreError::Corrupt("the OS random source failed".into()));
        }
    }
    let rounds = round_robin(teams.len(), settings.legs);
    let duration_ms = i64::from(settings.match_minutes) * 60_000;
    for plan in plan_slots(&rounds, courts.len()) {
        sqlx::query(
            "INSERT INTO non_stop_slots (event_id, slot, round, wave, state, duration_ms, \
             remaining_ms) VALUES (?, ?, ?, ?, 'ready', ?, ?)",
        )
        .bind(&ev.id)
        .bind(plan.slot)
        .bind(plan.round)
        .bind(plan.wave)
        .bind(duration_ms)
        .bind(duration_ms)
        .execute(&mut **tx)
        .await?;
        for (pairing, court_idx) in &plan.matches {
            let team_of = |i: usize| {
                order
                    .get(i)
                    .and_then(|&t| teams.get(t))
                    .ok_or_else(|| StoreError::Corrupt("schedule names a missing team".into()))
            };
            let (a, b) = (team_of(pairing.a)?, team_of(pairing.b)?);
            let court = courts
                .get(*court_idx)
                .ok_or_else(|| StoreError::Corrupt("schedule names a missing court".into()))?;
            let label = format!("Round {}, match {}", pairing.round, pairing.index);
            let row =
                create_event_match_in(tx, ev, &a.id, &b.id, Some(label), Some(&court.id), now)
                    .await?;
            sqlx::query(
                "INSERT INTO non_stop_matches (match_id, event_id, slot, leg) VALUES (?, ?, ?, ?)",
            )
            .bind(&row.match_id)
            .bind(&ev.id)
            .bind(plan.slot)
            .bind(pairing.leg + 1)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

/// A match of a done slot that has lost (or never had) its result is flagged for the runner
/// to call time on it again. On a finished event, the corrected standings rewrite the placings.
pub async fn on_result(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE non_stop_matches SET needs_time_up = 1 WHERE match_id = ? \
         AND EXISTS (SELECT 1 FROM non_stop_slots s WHERE s.event_id = non_stop_matches.event_id \
           AND s.slot = non_stop_matches.slot AND s.state = 'done') \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = ?)",
    )
    .bind(match_id)
    .bind(match_id)
    .execute(&mut **tx)
    .await?;
    if ev.status == Status::Finished {
        let teams = teams_in(tx, &ev.id).await?;
        let places = places_of(&standings_in(tx, &ev.id, &teams).await?);
        write_placings_in(tx, ev, &teams, &places, now).await?;
    }
    Ok(())
}

/// Whether this kind works out the final places itself (`computed_places` returns `Some`).
pub fn computes_places() -> bool {
    true
}

pub async fn computed_places(
    conn: &mut SqliteConnection,
    ev: &Competition,
) -> Result<Option<Vec<Place>>, StoreError> {
    let teams = teams_in(conn, &ev.id).await?;
    Ok(Some(places_of(&standings_in(conn, &ev.id, &teams).await?)))
}

/// The event's standings from its results so far (unplayed matches count for nothing).
async fn standings_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    teams: &[EventTeam],
) -> Result<Vec<StandingRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT c.team_a_id, c.team_b_id, r.winner, r.score FROM match_results r \
         JOIN competition_matches c ON c.match_id = r.match_id \
         JOIN non_stop_matches n ON n.match_id = r.match_id \
         WHERE n.event_id = ? ORDER BY c.ordinal",
    )
    .bind(event_id)
    .fetch_all(conn)
    .await?;
    let mut outcomes = Vec::with_capacity(rows.len());
    for row in &rows {
        let score: FinalScore = serde_json::from_str(row.try_get::<&str, _>("score")?)
            .map_err(|e| StoreError::Corrupt(format!("match result score: {e}")))?;
        let winner = match row.try_get::<Option<&str>, _>("winner")? {
            Some("a") => Some(Team::A),
            Some("b") => Some(Team::B),
            None => None,
            Some(other) => return Err(StoreError::Corrupt(format!("result winner {other:?}"))),
        };
        outcomes.push(Outcome {
            a: row.try_get("team_a_id")?,
            b: row.try_get("team_b_id")?,
            winner,
            games: score.regulation_games,
        });
    }
    let ids: Vec<String> = teams.iter().map(|t| t.id.clone()).collect();
    Ok(standings::standings(&ids, &outcomes))
}

/// `(rounds, waves)`: the schedule's largest round and wave numbers (0 before start).
async fn shape_in(conn: &mut SqliteConnection, event_id: &str) -> Result<(i64, i64), StoreError> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(MAX(round), 0), COALESCE(MAX(wave), 0) FROM non_stop_slots \
         WHERE event_id = ?",
    )
    .bind(event_id)
    .fetch_one(conn)
    .await?)
}

/// The non-stop's part of the event view: see the plan's wire shapes. The coordinator and
/// spectators get the same (it holds no token); a court gets the clock of its match.
pub async fn kind_data(
    conn: &mut SqliteConnection,
    ev: &Competition,
    audience: Audience,
) -> Result<Option<serde_json::Value>, StoreError> {
    let now_ms = system_now_ms();
    let value = match audience {
        Audience::Court { court_id } => {
            let current: Option<Option<String>> = sqlx::query_scalar(
                "SELECT current_match_id FROM courts WHERE id = ? AND event_id = ?",
            )
            .bind(&court_id)
            .bind(&ev.id)
            .fetch_optional(&mut *conn)
            .await?;
            let match_id = match current {
                None => None,
                Some(Some(id)) => Some(id),
                Some(None) => next_queued_in(conn, &court_id).await?,
            };
            // A finished event's runner has stopped, so no clock is shown.
            let clock = match match_id {
                Some(id) if ev.status == Status::Running => {
                    slot_of_match_in(conn, &id).await?.map(|s| s.clock(now_ms))
                }
                _ => None,
            };
            let (rounds, waves) = shape_in(conn, &ev.id).await?;
            json!({ "clock": clock, "rounds": rounds, "waves": waves })
        }
        Audience::Coordinator | Audience::Spectator => {
            let slots = slots_in(conn, &ev.id).await?;
            let links: Vec<(i64, String)> = sqlx::query_as(
                "SELECT n.slot, n.match_id FROM non_stop_matches n \
                 JOIN competition_matches c ON c.match_id = n.match_id \
                 WHERE n.event_id = ? ORDER BY c.ordinal",
            )
            .bind(&ev.id)
            .fetch_all(&mut *conn)
            .await?;
            let slot_list: Vec<serde_json::Value> = slots
                .iter()
                .map(|s| {
                    let matches: Vec<&str> = links
                        .iter()
                        .filter(|(slot, _)| *slot == i64::from(s.slot))
                        .map(|(_, id)| id.as_str())
                        .collect();
                    json!({
                        "slot": s.slot, "round": s.round, "wave": s.wave,
                        "state": s.state, "matches": matches,
                    })
                })
                .collect();
            let teams = teams_in(conn, &ev.id).await?;
            let standings = standings_in(conn, &ev.id, &teams).await?;
            json!({
                "settings": settings_of(ev)?,
                "rounds": slots.iter().map(|s| s.round).max().unwrap_or(0),
                "waves": slots.iter().map(|s| s.wave).max().unwrap_or(0),
                "clock": current_clock(&slots)
                    .filter(|_| ev.status == Status::Running)
                    .map(|s| s.clock(now_ms)),
                "slots": slot_list,
                "standings": standings,
            })
        }
    };
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::shuffle;

    fn counter(start: u64) -> impl FnMut() -> u64 {
        let mut x = start;
        move || {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            x >> 17
        }
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut v: Vec<usize> = (0..8).collect();
        shuffle(&mut v, counter(7));
        assert_ne!(v, (0..8).collect::<Vec<_>>());
        v.sort_unstable();
        assert_eq!(v, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_with_a_constant_source_is_deterministic() {
        let run = || {
            let mut v: Vec<usize> = (0..8).collect();
            shuffle(&mut v, || 0);
            v
        };
        assert_eq!(run(), run());
        let mut sorted = run();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_handles_tiny_slices() {
        let mut empty: [usize; 0] = [];
        shuffle(&mut empty, || 5);
        let mut one = [0usize];
        shuffle(&mut one, || 5);
        assert_eq!(one, [0]);
    }
}
