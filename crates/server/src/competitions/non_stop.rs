//! The non-stop kind. Owned by phase 8: phase 7 never edits this file. The phase 6 stubs
//! below do nothing; phase 8 fills them in.

mod schedule;
mod standings;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use super::courts::{courts_in, create_event_match_in};
use super::places::Place;
use super::{Audience, Competition, EventTeam, teams_in};
use crate::store::{StoreError, Tx};
use schedule::{plan_slots, round_robin};

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
