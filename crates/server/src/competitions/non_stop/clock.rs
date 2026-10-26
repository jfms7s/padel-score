//! The non-stop clock: one slot (a wave of a round) runs at a time. The coordinator's taps
//! (`clock_action`) move a slot between `ready`, `running`, `paused` and `done`; the runner
//! (`non_stop_api::run_clock`) uses the `*_in` helpers below to clear practice points at the go
//! and call time at the end. Clock times are unix milliseconds.

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::competitions::courts::{advance_court_in, courts_in, next_queued_in};
use crate::competitions::{Competition, Kind, Status, competition_in};
use crate::store::{Store, StoreError, Tx};

/// Start and Resume both count in for this long before the clock moves.
pub const COUNT_IN_MS: i64 = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockAction {
    Start,
    Pause,
    Resume,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotState {
    Ready,
    Running,
    Paused,
    Done,
}

impl SlotState {
    fn parse(s: &str) -> Result<SlotState, sqlx::Error> {
        match s {
            "ready" => Ok(SlotState::Ready),
            "running" => Ok(SlotState::Running),
            "paused" => Ok(SlotState::Paused),
            "done" => Ok(SlotState::Done),
            other => Err(sqlx::Error::Decode(
                format!("unknown slot state {other:?}").into(),
            )),
        }
    }
}

/// A slot's clock as the phones get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Clock {
    pub slot: u32,
    pub round: u32,
    pub wave: u32,
    pub state: SlotState,
    pub duration_ms: i64,
    /// Running: as of the read (the full time during the count-in).
    pub remaining_ms: i64,
    /// Running only; in the future during the count-in.
    pub starts_at_ms: Option<i64>,
    /// Running only.
    pub ends_at_ms: Option<i64>,
    /// The first go has happened (practice points are cleared); false for a ready slot and
    /// for one paused or ended before its first go.
    pub went: bool,
}

/// A `non_stop_slots` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotRow {
    pub slot: u32,
    pub round: u32,
    pub wave: u32,
    pub state: SlotState,
    pub duration_ms: i64,
    pub remaining_ms: i64,
    pub starts_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
    pub went: bool,
}

impl SlotRow {
    /// The clock as of `now_ms`: a running slot's time left is what was left at the tap, or
    /// less once its count-in is over.
    pub fn clock(&self, now_ms: i64) -> Clock {
        let remaining_ms = match (self.state, self.ends_at_ms) {
            (SlotState::Running, Some(ends)) => {
                self.remaining_ms.min(ends.saturating_sub(now_ms).max(0))
            }
            _ => self.remaining_ms,
        };
        Clock {
            slot: self.slot,
            round: self.round,
            wave: self.wave,
            state: self.state,
            duration_ms: self.duration_ms,
            remaining_ms,
            starts_at_ms: self.starts_at_ms,
            ends_at_ms: self.ends_at_ms,
            went: self.went,
        }
    }
}

fn u32_of(row: &SqliteRow, col: &str) -> Result<u32, sqlx::Error> {
    let v: i64 = row.try_get(col)?;
    u32::try_from(v).map_err(|e| sqlx::Error::Decode(Box::new(e)))
}

fn slot_from_row(row: &SqliteRow) -> Result<SlotRow, sqlx::Error> {
    Ok(SlotRow {
        slot: u32_of(row, "slot")?,
        round: u32_of(row, "round")?,
        wave: u32_of(row, "wave")?,
        state: SlotState::parse(row.try_get::<&str, _>("state")?)?,
        duration_ms: row.try_get("duration_ms")?,
        remaining_ms: row.try_get("remaining_ms")?,
        starts_at_ms: row.try_get("starts_at_ms")?,
        ends_at_ms: row.try_get("ends_at_ms")?,
        went: row.try_get("went")?,
    })
}

macro_rules! select_slots {
    ($rest:literal) => {
        concat!(
            "SELECT s.slot, s.round, s.wave, s.state, s.duration_ms, s.remaining_ms, \
             s.starts_at_ms, s.ends_at_ms, s.started_at_ms IS NOT NULL AS went \
             FROM non_stop_slots s ",
            $rest
        )
    };
}

/// The event's slots in play order.
pub(crate) async fn slots_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Vec<SlotRow>, StoreError> {
    let rows = sqlx::query(select_slots!("WHERE s.event_id = ? ORDER BY s.slot"))
        .bind(event_id)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(slot_from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

/// The slot `match_id` is played in, if it is a non-stop match.
pub(crate) async fn slot_of_match_in(
    conn: &mut SqliteConnection,
    match_id: &str,
) -> Result<Option<SlotRow>, StoreError> {
    let row = sqlx::query(select_slots!(
        "JOIN non_stop_matches n ON n.event_id = s.event_id AND n.slot = s.slot \
         WHERE n.match_id = ?"
    ))
    .bind(match_id)
    .fetch_optional(conn)
    .await?;
    Ok(row.as_ref().map(slot_from_row).transpose()?)
}

/// The clock an event shows: the running or paused slot, else the lowest ready one.
pub(crate) fn current_clock(slots: &[SlotRow]) -> Option<&SlotRow> {
    slots
        .iter()
        .find(|s| matches!(s.state, SlotState::Running | SlotState::Paused))
        .or_else(|| slots.iter().find(|s| s.state == SlotState::Ready))
}

/// The current system time in unix ms (0 before the epoch).
pub(crate) fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

async fn set_running_in(
    tx: &mut Tx<'_>,
    event_id: &str,
    slot: &SlotRow,
    now_ms: i64,
) -> Result<(), StoreError> {
    let starts = now_ms.saturating_add(COUNT_IN_MS);
    sqlx::query(
        "UPDATE non_stop_slots SET state = 'running', starts_at_ms = ?, ends_at_ms = ? \
         WHERE event_id = ? AND slot = ?",
    )
    .bind(starts)
    .bind(starts.saturating_add(slot.remaining_ms))
    .bind(event_id)
    .bind(slot.slot)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Advance every court whose current match is finished (or that has none) and whose next
/// queued match is one of `slot`'s, so that wave's matches become current. A court idle in this
/// wave keeps its finished match (and its token) until a slot of its own starts.
async fn advance_finished_courts_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    slot: u32,
) -> Result<(), StoreError> {
    for court in courts_in(tx, &ev.id).await? {
        let free = match &court.current_match_id {
            None => true,
            Some(current) => {
                let found: Option<i64> =
                    sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
                        .bind(current)
                        .fetch_optional(&mut **tx)
                        .await?;
                found.is_some()
            }
        };
        if !free {
            continue;
        }
        let Some(next) = next_queued_in(tx, &court.id).await? else {
            continue;
        };
        let in_slot: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM non_stop_matches WHERE match_id = ? AND event_id = ? AND slot = ?",
        )
        .bind(&next)
        .bind(&ev.id)
        .bind(slot)
        .fetch_optional(&mut **tx)
        .await?;
        if in_slot.is_some() {
            advance_court_in(tx, ev, &court.id).await?;
        }
    }
    Ok(())
}

impl Store {
    /// The coordinator's clock tap on `slot` (the slot their screen showed, so a stale or
    /// doubled tap is refused). One `BEGIN IMMEDIATE`.
    pub async fn clock_action(
        &self,
        owner: &str,
        event_id: &str,
        action: ClockAction,
        slot: u32,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if ev.kind != Kind::NonStop {
            return Err(StoreError::Conflict("not_non_stop"));
        }
        if ev.status != Status::Running {
            return Err(StoreError::Conflict("event_not_running"));
        }
        let slots = slots_in(&mut tx, event_id).await?;
        let target = slots.iter().find(|s| s.slot == slot);
        let state = target.map(|s| s.state);
        match action {
            ClockAction::Start => {
                if slots
                    .iter()
                    .any(|s| matches!(s.state, SlotState::Running | SlotState::Paused))
                {
                    return Err(StoreError::Conflict("clock_busy"));
                }
                let next = slots.iter().find(|s| s.state == SlotState::Ready);
                let Some(target) = next.filter(|s| s.slot == slot) else {
                    return Err(StoreError::Conflict("not_next_slot"));
                };
                let unfinished: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM non_stop_matches n WHERE n.event_id = ? \
                     AND n.slot < ? AND NOT EXISTS \
                     (SELECT 1 FROM match_results r WHERE r.match_id = n.match_id))",
                )
                .bind(event_id)
                .bind(slot)
                .fetch_one(&mut *tx)
                .await?;
                if unfinished {
                    return Err(StoreError::Conflict("previous_unfinished"));
                }
                set_running_in(&mut tx, event_id, target, now_ms).await?;
                advance_finished_courts_in(&mut tx, &ev, slot).await?;
            }
            ClockAction::Pause => {
                let Some(target) = target.filter(|s| s.state == SlotState::Running) else {
                    return Err(StoreError::Conflict("clock_not_running"));
                };
                let (Some(starts), Some(ends)) = (target.starts_at_ms, target.ends_at_ms) else {
                    return Err(StoreError::Corrupt("a running slot has no times".into()));
                };
                // Time is over and the runner has not called it yet: nothing left to pause.
                if ends <= now_ms {
                    return Err(StoreError::Conflict("clock_not_running"));
                }
                let left = ends
                    .saturating_sub(now_ms.max(starts))
                    .clamp(0, target.remaining_ms.max(0));
                sqlx::query(
                    "UPDATE non_stop_slots SET state = 'paused', remaining_ms = ?, \
                     starts_at_ms = NULL, ends_at_ms = NULL WHERE event_id = ? AND slot = ?",
                )
                .bind(left)
                .bind(event_id)
                .bind(slot)
                .execute(&mut *tx)
                .await?;
            }
            ClockAction::Resume => {
                let Some(target) = target.filter(|s| s.state == SlotState::Paused) else {
                    return Err(StoreError::Conflict("clock_not_paused"));
                };
                set_running_in(&mut tx, event_id, target, now_ms).await?;
            }
            ClockAction::End => {
                if !matches!(state, Some(SlotState::Running | SlotState::Paused)) {
                    return Err(StoreError::Conflict("clock_not_running"));
                }
                mark_done_in(&mut tx, event_id, slot, now_ms).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Slots whose go is due: of running events, not gone yet (`started_at_ms` empty), and either
/// `running` with the count-in over or `done` (ended before the go). The runner clears their
/// practice points, then calls [`went_in`].
pub async fn due_go_in(
    conn: &mut SqliteConnection,
    now_ms: i64,
) -> Result<Vec<(String, u32)>, StoreError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.event_id, s.slot FROM non_stop_slots s \
         JOIN competitions c ON c.id = s.event_id \
         WHERE c.status = 'running' AND s.started_at_ms IS NULL \
         AND ((s.state = 'running' AND s.starts_at_ms <= ?) OR s.state = 'done') \
         ORDER BY s.event_id, s.slot",
    )
    .bind(now_ms)
    .fetch_all(conn)
    .await?;
    event_slots(rows)
}

/// The slot's first go: set once, so a Resume never clears again.
pub async fn went_in(
    tx: &mut Tx<'_>,
    event_id: &str,
    slot: u32,
    now_ms: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE non_stop_slots SET started_at_ms = ? \
         WHERE event_id = ? AND slot = ? AND started_at_ms IS NULL",
    )
    .bind(now_ms)
    .bind(event_id)
    .bind(slot)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Running slots of running events whose time is up.
pub async fn due_slots_in(
    conn: &mut SqliteConnection,
    now_ms: i64,
) -> Result<Vec<(String, u32)>, StoreError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.event_id, s.slot FROM non_stop_slots s \
         JOIN competitions c ON c.id = s.event_id \
         WHERE s.state = 'running' AND s.ends_at_ms <= ? AND c.status = 'running' \
         ORDER BY s.event_id, s.slot",
    )
    .bind(now_ms)
    .fetch_all(conn)
    .await?;
    event_slots(rows)
}

fn event_slots(rows: Vec<(String, i64)>) -> Result<Vec<(String, u32)>, StoreError> {
    rows.into_iter()
        .map(|(event, slot)| {
            u32::try_from(slot)
                .map(|slot| (event, slot))
                .map_err(|_| StoreError::Corrupt("slot number out of range".into()))
        })
        .collect()
}

/// The slot is over at `now_ms`: `done`, no time left, and every match of it flagged for the
/// runner to call time on (a finished one is skipped there).
pub async fn mark_done_in(
    tx: &mut Tx<'_>,
    event_id: &str,
    slot: u32,
    now_ms: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE non_stop_slots SET state = 'done', done_at_ms = ?, remaining_ms = 0, \
         starts_at_ms = NULL, ends_at_ms = NULL WHERE event_id = ? AND slot = ?",
    )
    .bind(now_ms)
    .bind(event_id)
    .bind(slot)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE non_stop_matches SET needs_time_up = 1 WHERE event_id = ? AND slot = ?")
        .bind(event_id)
        .bind(slot)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Flagged matches of running events whose slot has gone, for the runner to check their
/// time-up. A slot ended before its go is left out until [`went_in`], so time is never called
/// on a log that still holds practice points.
pub async fn pending_time_up_in(conn: &mut SqliteConnection) -> Result<Vec<String>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT n.match_id FROM non_stop_matches n JOIN competitions c ON c.id = n.event_id \
         JOIN non_stop_slots s ON s.event_id = n.event_id AND s.slot = n.slot \
         WHERE n.needs_time_up = 1 AND c.status = 'running' AND s.started_at_ms IS NOT NULL \
         ORDER BY n.match_id",
    )
    .fetch_all(conn)
    .await?)
}

pub async fn clear_time_up_in(
    conn: &mut SqliteConnection,
    match_id: &str,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE non_stop_matches SET needs_time_up = 0 WHERE match_id = ?")
        .bind(match_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// The runner's next wake-up: the earliest go of a running slot that hasn't gone, or end of a
/// running slot, across running events.
pub async fn next_deadline_in(conn: &mut SqliteConnection) -> Result<Option<i64>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT MIN(CASE WHEN s.started_at_ms IS NULL THEN s.starts_at_ms ELSE s.ends_at_ms END) \
         FROM non_stop_slots s \
         JOIN competitions c ON c.id = s.event_id \
         WHERE s.state = 'running' AND c.status = 'running'",
    )
    .fetch_one(conn)
    .await?)
}

/// Whether any non-stop event is running (the runner's once-a-second re-check).
pub async fn any_running_non_stop_in(conn: &mut SqliteConnection) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM competitions WHERE kind = 'non_stop' AND status = 'running')",
    )
    .fetch_one(conn)
    .await?)
}
