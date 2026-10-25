//! The command rules: live points, offline batches, undo, replace and idempotency.
//!
//! Every request runs in one write transaction and is all-or-nothing. Inside it the match and
//! its log are always re-read; the `&StoredMatch` a caller passes in only names the match.
//!
//! Outcomes and their HTTP shape ([`CommandOutcome::to_http`] is the single source of truth, so a
//! fresh answer and a replayed one are byte-for-byte the same):
//!
//! | Outcome    | Status | Body                                               | Saved for `client_id` |
//! |------------|--------|----------------------------------------------------|-----------------------|
//! | `Applied`  | 200    | `MatchView`                                        | yes                   |
//! | `Conflict` | 409    | `{"error":"conflict","view":MatchView}`            | no (depends on state) |
//! | `Rejected` | 422    | `{"error":"rejected","kind":ErrorKind,"index":n}`  | yes                   |
//! | `Invalid`  | 400    | `{"error":"invalid","message":"..."}`              | no                    |
//! | `Replayed` | stored | the stored body, verbatim                          | (it is the saved one) |
//!
//! A resend whose result was saved comes back as `Replayed { status, body }`; serve it as is.

use padel_scoring::{ErrorKind, Event, Format, ReplayError, Score, Start, Team, replay};
use serde::{Deserialize, Serialize};

use crate::store::{
    EventState, Store, StoreError, StoredEvent, StoredMatch, Teams, Tx, append, events_in,
    find_result_in, mark, match_by_id_in, save_result, touch,
};

/// The most commands one request may carry.
pub const MAX_COMMANDS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    Point {
        team: Team,
    },
    TimeUp,
    /// Undo the latest effective event, which must be `target_seq`.
    Undo {
        target_seq: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub client_id: String,
    /// A batch (queued offline) must be based on the current revision; a live command need not.
    pub batch: bool,
    /// Binding for a batch, informational for a live command.
    pub base_revision: i64,
    pub commands: Vec<Command>,
}

/// "Keep mine" after a conflict: drop the server's events after `base_seq` and apply mine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceRequest {
    pub client_id: String,
    pub base_revision: i64,
    pub base_seq: i64,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewEvent {
    pub seq: i64,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchView {
    pub id: String,
    pub format: Format,
    pub start: Start,
    pub teams: Teams,
    pub revision: i64,
    /// The highest seq of any row, effective or not; 0 for an empty log.
    pub head_seq: i64,
    /// The effective events only, in seq order.
    pub events: Vec<ViewEvent>,
    pub score: Score,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Applied(MatchView),
    /// Nothing was written; the view is the current state.
    Conflict(MatchView),
    /// The engine refused the command at `index` within this request's `commands`. Nothing was
    /// written to the log.
    Rejected {
        kind: ErrorKind,
        index: usize,
    },
    /// A malformed request (no commands, too many, or no `client_id`).
    Invalid(&'static str),
    /// A resend: the saved result of the first request with this `client_id`.
    Replayed {
        status: u16,
        body: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum ErrorBody<'a> {
    Conflict { view: &'a MatchView },
    Rejected { kind: ErrorKind, index: usize },
    Invalid { message: &'a str },
}

impl CommandOutcome {
    /// The HTTP status and JSON body for this outcome.
    pub fn to_http(&self) -> Result<(u16, String), serde_json::Error> {
        Ok(match self {
            CommandOutcome::Applied(view) => (200, serde_json::to_string(view)?),
            CommandOutcome::Conflict(view) => {
                (409, serde_json::to_string(&ErrorBody::Conflict { view })?)
            }
            CommandOutcome::Rejected { kind, index } => (
                422,
                serde_json::to_string(&ErrorBody::Rejected {
                    kind: *kind,
                    index: *index,
                })?,
            ),
            CommandOutcome::Invalid(message) => {
                (400, serde_json::to_string(&ErrorBody::Invalid { message })?)
            }
            CommandOutcome::Replayed { status, body } => (*status, body.clone()),
        })
    }
}

/// The match as clients see it: the score is a replay of the effective events.
pub fn view_of(m: &StoredMatch, events: &[StoredEvent]) -> Result<MatchView, ReplayError> {
    let effective: Vec<ViewEvent> = events
        .iter()
        .filter(|e| e.state == EventState::Effective)
        .map(|e| ViewEvent {
            seq: e.seq,
            event: e.event,
        })
        .collect();
    let played: Vec<Event> = effective.iter().map(|e| e.event).collect();
    let score = replay(&m.format, &m.start, &played)?;
    Ok(MatchView {
        id: m.id.clone(),
        format: m.format,
        start: m.start,
        teams: m.teams.clone(),
        revision: m.revision,
        head_seq: head_seq_of(events),
        events: effective,
        score,
    })
}

/// Apply a live command or an offline batch. See the module docs for the outcomes.
pub async fn apply_command(
    store: &Store,
    m: &StoredMatch,
    req: CommandRequest,
    now: i64,
) -> Result<CommandOutcome, StoreError> {
    let guard = if req.batch {
        Guard::Batch {
            base_revision: req.base_revision,
        }
    } else {
        Guard::Live
    };
    execute(store, &m.id, &req.client_id, guard, &req.commands, now).await
}

/// Mark the effective events after `base_seq` as replaced, then apply `commands` as live
/// appends (new seqs continue past `head_seq`; seqs are never reused).
pub async fn replace_from(
    store: &Store,
    m: &StoredMatch,
    req: ReplaceRequest,
    now: i64,
) -> Result<CommandOutcome, StoreError> {
    let guard = Guard::Replace {
        base_revision: req.base_revision,
        base_seq: req.base_seq,
    };
    execute(store, &m.id, &req.client_id, guard, &req.commands, now).await
}

enum Guard {
    Live,
    Batch { base_revision: i64 },
    Replace { base_revision: i64, base_seq: i64 },
}

/// One event of the candidate effective list. `origin` is the index of the command that adds it
/// in this request, or `None` for an event already stored.
struct Candidate {
    seq: i64,
    event: Event,
    origin: Option<usize>,
}

/// The writes a request would make, once its undos are resolved.
struct Plan {
    candidate: Vec<Candidate>,
    appended: Vec<(i64, Event)>,
    undone: Vec<i64>,
}

fn head_seq_of(events: &[StoredEvent]) -> i64 {
    events.iter().map(|e| e.seq).max().unwrap_or(0)
}

fn corrupt(e: ReplayError) -> StoreError {
    StoreError::Corrupt(format!("stored effective events do not replay: {e}"))
}

fn invalid(client_id: &str, commands: &[Command]) -> Option<&'static str> {
    if client_id.trim().is_empty() {
        Some("client_id is required")
    } else if commands.is_empty() {
        Some("commands must not be empty")
    } else if commands.len() > MAX_COMMANDS {
        Some("too many commands in one request")
    } else {
        None
    }
}

/// Resolve `commands` against the effective events, in order. `None` means an undo did not name
/// the latest effective event at its point in the request: a conflict.
fn plan(effective: Vec<Candidate>, head_seq: i64, commands: &[Command]) -> Option<Plan> {
    let mut plan = Plan {
        candidate: effective,
        appended: Vec::new(),
        undone: Vec::new(),
    };
    let mut next_seq = head_seq + 1;
    for (index, command) in commands.iter().enumerate() {
        let event = match command {
            Command::Point { team } => Event::Point(*team),
            Command::TimeUp => Event::TimeUp,
            Command::Undo { target_seq } => {
                if plan.candidate.last().map(|c| c.seq) != Some(*target_seq) {
                    return None;
                }
                plan.candidate.pop();
                plan.undone.push(*target_seq);
                continue;
            }
        };
        plan.candidate.push(Candidate {
            seq: next_seq,
            event,
            origin: Some(index),
        });
        plan.appended.push((next_seq, event));
        next_seq += 1;
    }
    Some(plan)
}

async fn save(
    tx: &mut Tx<'_>,
    match_id: &str,
    client_id: &str,
    outcome: &CommandOutcome,
) -> Result<(), StoreError> {
    let (status, body) = outcome
        .to_http()
        .map_err(|e| StoreError::Db(sqlx::Error::Encode(Box::new(e))))?;
    save_result(tx, match_id, client_id, status, &body).await
}

async fn execute(
    store: &Store,
    match_id: &str,
    client_id: &str,
    guard: Guard,
    commands: &[Command],
    now: i64,
) -> Result<CommandOutcome, StoreError> {
    if let Some(message) = invalid(client_id, commands) {
        return Ok(CommandOutcome::Invalid(message));
    }
    // Dropping `tx` without a commit rolls it back: every early return below writes nothing.
    let mut tx = store.begin().await?;
    if let Some(saved) = find_result_in(&mut tx, match_id, client_id).await? {
        return Ok(CommandOutcome::Replayed {
            status: saved.status,
            body: saved.body,
        });
    }
    let m = match_by_id_in(&mut tx, match_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let events = events_in(&mut tx, match_id).await?;
    let head_seq = head_seq_of(&events);
    let conflict = || -> Result<CommandOutcome, StoreError> {
        Ok(CommandOutcome::Conflict(
            view_of(&m, &events).map_err(corrupt)?,
        ))
    };

    let replaced: Vec<i64> = match guard {
        Guard::Live => Vec::new(),
        Guard::Batch { base_revision } => {
            if base_revision != m.revision {
                return conflict();
            }
            Vec::new()
        }
        Guard::Replace {
            base_revision,
            base_seq,
        } => {
            if base_revision != m.revision || base_seq < 0 || base_seq > head_seq {
                return conflict();
            }
            events
                .iter()
                .filter(|e| e.state == EventState::Effective && e.seq > base_seq)
                .map(|e| e.seq)
                .collect()
        }
    };

    let effective = events
        .iter()
        .filter(|e| e.state == EventState::Effective && !replaced.contains(&e.seq))
        .map(|e| Candidate {
            seq: e.seq,
            event: e.event,
            origin: None,
        })
        .collect();
    let Some(plan) = plan(effective, head_seq, commands) else {
        return conflict();
    };

    let played: Vec<Event> = plan.candidate.iter().map(|c| c.event).collect();
    if let Err(e) = replay(&m.format, &m.start, &played) {
        let origin = plan.candidate.get(e.index).and_then(|c| c.origin);
        return match (e.kind, origin) {
            // Rules are validated when the match is created, and the stored events replayed
            // cleanly before this request: either failing here means the log is broken.
            (ErrorKind::InvalidRules(_), _) | (_, None) => Err(corrupt(e)),
            (kind, Some(index)) => {
                let outcome = CommandOutcome::Rejected { kind, index };
                save(&mut tx, match_id, client_id, &outcome).await?;
                tx.commit().await?;
                Ok(outcome)
            }
        };
    }

    mark(&mut tx, match_id, &replaced, EventState::Replaced).await?;
    for (seq, event) in &plan.appended {
        append(&mut tx, match_id, *seq, *event, client_id, now).await?;
    }
    // An undo may target a row appended just above, so mark after appending.
    mark(&mut tx, match_id, &plan.undone, EventState::Undone).await?;
    touch(&mut tx, match_id, now).await?;

    let m = match_by_id_in(&mut tx, match_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let events = events_in(&mut tx, match_id).await?;
    let outcome = CommandOutcome::Applied(view_of(&m, &events).map_err(corrupt)?);
    save(&mut tx, match_id, client_id, &outcome).await?;
    tx.commit().await?;
    Ok(outcome)
}
