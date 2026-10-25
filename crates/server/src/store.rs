//! The match store: matches, the append-only event log, and stored command results.
//!
//! The store keeps events, never engine state. Undo, replace and discard mark rows; they never
//! delete them. The clock is never read here: callers pass `now` in unix seconds.
//!
//! Commands run inside a transaction from [`Store::begin`], which takes SQLite's write lock up
//! front so two commands on one match cannot interleave. Reads that belong to a command (the
//! `*_in` functions) must go through the same transaction.

use std::fmt;

use padel_scoring::{Event, Format, RulesError, Start};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::tokens::new_token;

pub type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

const MAX_NAME_CHARS: usize = 40;

#[derive(Debug)]
pub enum StoreError {
    InvalidRules(RulesError),
    InvalidTeams,
    NotFound,
    /// The stored log contradicts itself, e.g. the engine rejects an event already stored as
    /// effective. Never expected; the request is refused rather than shown a wrong score.
    Corrupt(String),
    Db(sqlx::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::InvalidRules(e) => write!(f, "invalid rules: {e}"),
            StoreError::InvalidTeams => write!(f, "invalid team names"),
            StoreError::NotFound => write!(f, "not found"),
            StoreError::Corrupt(msg) => write!(f, "corrupt match log: {msg}"),
            StoreError::Db(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        StoreError::Db(e)
    }
}

impl From<RulesError> for StoreError {
    fn from(e: RulesError) -> Self {
        StoreError::InvalidRules(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Teams {
    pub a: [String; 2],
    pub b: [String; 2],
}

impl Teams {
    /// Trim every name; a blank one becomes "Player 1" / "Player 2" by its position in the team.
    /// A name over 40 chars (not bytes) is refused.
    pub fn normalized(&self) -> Result<Teams, StoreError> {
        fn side(names: &[String; 2]) -> Result<[String; 2], StoreError> {
            let one = |name: &str, fallback: &str| {
                let name = name.trim();
                if name.is_empty() {
                    Ok(fallback.to_owned())
                } else if name.chars().count() > MAX_NAME_CHARS {
                    Err(StoreError::InvalidTeams)
                } else {
                    Ok(name.to_owned())
                }
            };
            let [first, second] = names;
            Ok([one(first, "Player 1")?, one(second, "Player 2")?])
        }
        Ok(Teams {
            a: side(&self.a)?,
            b: side(&self.b)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMatch {
    pub format: Format,
    pub start: Start,
    pub teams: Teams,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMatch {
    pub id: String,
    pub scorer_token: String,
    pub spectator_token: String,
    pub format: Format,
    pub start: Start,
    pub teams: Teams,
    pub revision: i64,
    pub last_activity_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventState {
    Effective,
    Undone,
    Discarded,
    Replaced,
}

impl EventState {
    pub fn as_str(self) -> &'static str {
        match self {
            EventState::Effective => "effective",
            EventState::Undone => "undone",
            EventState::Discarded => "discarded",
            EventState::Replaced => "replaced",
        }
    }

    fn parse(s: &str) -> Result<EventState, sqlx::Error> {
        match s {
            "effective" => Ok(EventState::Effective),
            "undone" => Ok(EventState::Undone),
            "discarded" => Ok(EventState::Discarded),
            "replaced" => Ok(EventState::Replaced),
            other => Err(decode_error(format!("unknown event state {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub seq: i64,
    pub event: Event,
    pub client_id: String,
    pub state: EventState,
}

/// A command's stored outcome, replayed verbatim when its `client_id` is resent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResult {
    pub status: u16,
    pub body: String,
}

fn decode_error(msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> sqlx::Error {
    sqlx::Error::Decode(msg.into())
}

fn json_error(e: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(e))
}

fn encode<T: Serialize>(value: &T) -> Result<String, sqlx::Error> {
    serde_json::to_string(value).map_err(json_error)
}

fn decode<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, sqlx::Error> {
    serde_json::from_str(text).map_err(json_error)
}

fn match_from_row(row: &SqliteRow) -> Result<StoredMatch, sqlx::Error> {
    Ok(StoredMatch {
        id: row.try_get("id")?,
        scorer_token: row.try_get("scorer_token")?,
        spectator_token: row.try_get("spectator_token")?,
        format: decode(row.try_get::<&str, _>("format")?)?,
        start: decode(row.try_get::<&str, _>("start")?)?,
        teams: decode(row.try_get::<&str, _>("teams")?)?,
        revision: row.try_get("revision")?,
        last_activity_at: row.try_get("last_activity_at")?,
    })
}

fn event_from_row(row: &SqliteRow) -> Result<StoredEvent, sqlx::Error> {
    Ok(StoredEvent {
        seq: row.try_get("seq")?,
        event: decode(row.try_get::<&str, _>("event")?)?,
        client_id: row.try_get("client_id")?,
        state: EventState::parse(row.try_get::<&str, _>("state")?)?,
    })
}

// Reads that also run inside a command's transaction. A `&mut Tx` coerces to the connection.

pub async fn match_by_scorer_token_in(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<Option<StoredMatch>, StoreError> {
    let row = sqlx::query(
        "SELECT id, scorer_token, spectator_token, format, start, teams, revision, last_activity_at \
         FROM matches WHERE scorer_token = ?",
    )
    .bind(token)
    .fetch_optional(conn)
    .await?;
    Ok(row.as_ref().map(match_from_row).transpose()?)
}

pub async fn match_by_id_in(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<StoredMatch>, StoreError> {
    let row = sqlx::query(
        "SELECT id, scorer_token, spectator_token, format, start, teams, revision, \
         last_activity_at FROM matches WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    Ok(row.as_ref().map(match_from_row).transpose()?)
}

/// Every event of the match in seq order, whatever its state.
pub async fn events_in(
    conn: &mut SqliteConnection,
    match_id: &str,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT seq, event, client_id, state FROM events WHERE match_id = ? ORDER BY seq",
    )
    .bind(match_id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .iter()
        .map(event_from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

/// The highest seq in the log (any state), or 0 for an empty log.
pub async fn head_seq_in(conn: &mut SqliteConnection, match_id: &str) -> Result<i64, StoreError> {
    let head: Option<i64> = sqlx::query_scalar("SELECT MAX(seq) FROM events WHERE match_id = ?")
        .bind(match_id)
        .fetch_one(conn)
        .await?;
    Ok(head.unwrap_or(0))
}

pub async fn find_result_in(
    conn: &mut SqliteConnection,
    match_id: &str,
    client_id: &str,
) -> Result<Option<StoredResult>, StoreError> {
    let row = sqlx::query(
        "SELECT status, body FROM command_results WHERE match_id = ? AND client_id = ?",
    )
    .bind(match_id)
    .bind(client_id)
    .fetch_optional(conn)
    .await?;
    row.map(|r| -> Result<StoredResult, sqlx::Error> {
        let status: i64 = r.try_get("status")?;
        Ok(StoredResult {
            status: u16::try_from(status).map_err(|e| decode_error(e.to_string()))?,
            body: r.try_get("body")?,
        })
    })
    .transpose()
    .map_err(StoreError::from)
}

// Writes. They only run inside a transaction.

pub async fn append(
    tx: &mut Tx<'_>,
    match_id: &str,
    seq: i64,
    event: Event,
    client_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO events (match_id, seq, event, client_id, state, created_at) \
         VALUES (?, ?, ?, ?, 'effective', ?)",
    )
    .bind(match_id)
    .bind(seq)
    .bind(encode(&event)?)
    .bind(client_id)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Set the state of the given events. The rows stay.
pub async fn mark(
    tx: &mut Tx<'_>,
    match_id: &str,
    seqs: &[i64],
    state: EventState,
) -> Result<(), StoreError> {
    for seq in seqs {
        let done = sqlx::query("UPDATE events SET state = ? WHERE match_id = ? AND seq = ?")
            .bind(state.as_str())
            .bind(match_id)
            .bind(seq)
            .execute(&mut **tx)
            .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
    }
    Ok(())
}

/// Record that a command was applied: bump the revision and the last activity.
pub async fn touch(tx: &mut Tx<'_>, match_id: &str, now: i64) -> Result<(), StoreError> {
    let done = sqlx::query(
        "UPDATE matches SET revision = revision + 1, last_activity_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(match_id)
    .execute(&mut **tx)
    .await?;
    if done.rows_affected() != 1 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn save_result(
    tx: &mut Tx<'_>,
    match_id: &str,
    client_id: &str,
    status: u16,
    body: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO command_results (match_id, client_id, status, body) VALUES (?, ?, ?, ?)",
    )
    .bind(match_id)
    .bind(client_id)
    .bind(i64::from(status))
    .bind(body)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Store {
        Store { pool }
    }

    #[allow(dead_code)] // used by later tasks (retention, SSE)
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Start a write transaction. It takes the write lock immediately (`BEGIN IMMEDIATE`), so a
    /// command can read the log and append to it without another writer slipping in between.
    pub async fn begin(&self) -> Result<Tx<'static>, StoreError> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    pub async fn create_match(&self, new: NewMatch, now: i64) -> Result<StoredMatch, StoreError> {
        new.format.validate()?;
        new.start.validate()?;
        let teams = new.teams.normalized()?;
        let stored = StoredMatch {
            id: uuid::Uuid::new_v4().to_string(),
            scorer_token: new_token(),
            spectator_token: new_token(),
            format: new.format,
            start: new.start,
            teams,
            revision: 0,
            last_activity_at: now,
        };
        sqlx::query(
            "INSERT INTO matches (id, scorer_token, spectator_token, format, start, teams, \
             revision, created_at, last_activity_at) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&stored.id)
        .bind(&stored.scorer_token)
        .bind(&stored.spectator_token)
        .bind(encode(&stored.format)?)
        .bind(encode(&stored.start)?)
        .bind(encode(&stored.teams)?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(stored)
    }

    pub async fn by_scorer_token(&self, token: &str) -> Result<Option<StoredMatch>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        match_by_scorer_token_in(&mut conn, token).await
    }

    pub async fn by_spectator_token(&self, token: &str) -> Result<Option<StoredMatch>, StoreError> {
        let row = sqlx::query(
            "SELECT id, scorer_token, spectator_token, format, start, teams, revision, \
             last_activity_at FROM matches WHERE spectator_token = ?",
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(match_from_row).transpose()?)
    }

    /// The match and its whole log, read in one transaction so the two agree (the revision
    /// matches the events). `None` when the match does not exist.
    pub async fn snapshot(
        &self,
        match_id: &str,
    ) -> Result<Option<(StoredMatch, Vec<StoredEvent>)>, StoreError> {
        let mut tx = self.pool.begin().await?;
        let Some(m) = match_by_id_in(&mut tx, match_id).await? else {
            return Ok(None);
        };
        let events = events_in(&mut tx, match_id).await?;
        tx.commit().await?;
        Ok(Some((m, events)))
    }

    pub async fn events(&self, match_id: &str) -> Result<Vec<StoredEvent>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        events_in(&mut conn, match_id).await
    }

    pub async fn head_seq(&self, match_id: &str) -> Result<i64, StoreError> {
        let mut conn = self.pool.acquire().await?;
        head_seq_in(&mut conn, match_id).await
    }

    pub async fn find_result(
        &self,
        match_id: &str,
        client_id: &str,
    ) -> Result<Option<StoredResult>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        find_result_in(&mut conn, match_id, client_id).await
    }
}
