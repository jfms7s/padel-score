//! The match results projection: one `match_results` row per finished owned match.
//!
//! The event log stays the source of truth. A row is derived from the engine's `Score` and written
//! only here: by [`sync_in`] inside each applied command's transaction, so an undo or replace that
//! un-finishes a match retracts its row, and by the startup [`backfill`]. Anonymous matches never
//! get a row. The winner is read from `Score.status`, never worked out here.
//!
//! The read side is here too: the owner's [`Store::history`] and a player's
//! [`Store::player_history`] (keyset pages, newest first), win/loss/draw [`Record`]s and
//! [`Placing`]s. Every query is filtered by the caller's user id.

use std::collections::HashMap;
use std::fmt;

use padel_scoring::{Format, Score, SetScore, Status, Team};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::commands::view_of;
use crate::players::PlayerSlots;
use crate::store::{Store, StoreError, StoredMatch, Teams, Tx, events_in, match_by_id_in};

/// The final score as stored in `match_results.score` (JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalScore {
    /// The sets in order, as the engine reports them; empty for a timed match.
    pub sets: Vec<SetScore>,
    /// The final set's games; for a timed match, the whole match including any overtime game.
    pub games: [u16; 2],
    /// The games before any overtime game.
    pub regulation_games: [u16; 2],
    pub overtime: bool,
    pub timed: bool,
}

/// The winner (`None` for a timed draw) and final score, or `None` while the match is in progress.
pub fn result_of(format: &Format, score: &Score) -> Option<(Option<Team>, FinalScore)> {
    let Status::Finished { winner } = score.status else {
        return None;
    };
    Some((
        winner,
        FinalScore {
            sets: score.sets.clone(),
            games: score.games,
            regulation_games: score.regulation_games,
            overtime: score.overtime,
            timed: matches!(format, Format::Timed { .. }),
        },
    ))
}

fn team_text(team: Team) -> &'static str {
    match team {
        Team::A => "a",
        Team::B => "b",
    }
}

fn winner_text(winner: Option<Team>) -> Option<&'static str> {
    winner.map(team_text)
}

fn encode(score: &FinalScore) -> Result<String, StoreError> {
    serde_json::to_string(score).map_err(|e| StoreError::Db(sqlx::Error::Encode(Box::new(e))))
}

/// Make `match_results` agree with `score`, inside the command's transaction. An anonymous match
/// is left alone; a finished one is upserted with `finished_at = now`; any other is deleted.
pub async fn sync_in(
    tx: &mut Tx<'_>,
    m: &StoredMatch,
    score: &Score,
    now: i64,
) -> Result<(), StoreError> {
    let Some(owner) = m.owner_user_id.as_deref() else {
        return Ok(());
    };
    match result_of(&m.format, score) {
        Some((winner, final_score)) => {
            sqlx::query(
                "INSERT INTO match_results (match_id, owner_user_id, winner, score, finished_at) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(match_id) DO UPDATE SET winner = excluded.winner, \
                 score = excluded.score, finished_at = excluded.finished_at",
            )
            .bind(&m.id)
            .bind(owner)
            .bind(winner_text(winner))
            .bind(encode(&final_score)?)
            .bind(now)
            .execute(&mut **tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM match_results WHERE match_id = ?")
                .bind(&m.id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

/// Give every finished owned match that has no `match_results` row one, with
/// `finished_at = last_activity_at`. Returns how many rows it inserted; running it again inserts
/// none. Each match is replayed in its own write transaction, so a command cannot change it in
/// between. A match whose log does not replay is skipped with a warning.
pub async fn backfill(store: &Store) -> Result<usize, StoreError> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM matches WHERE owner_user_id IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = matches.id) \
         ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    let mut inserted = 0;
    for id in ids {
        let mut tx = store.begin().await?;
        let Some(m) = match_by_id_in(&mut tx, &id).await? else {
            continue;
        };
        let Some(owner) = m.owner_user_id.as_deref() else {
            continue;
        };
        let events = events_in(&mut tx, &id).await?;
        let view = match view_of(&m, &events) {
            Ok(view) => view,
            Err(error) => {
                tracing::warn!(match_id = %id, %error, "backfill: match log does not replay");
                continue;
            }
        };
        let Some((winner, final_score)) = result_of(&m.format, &view.score) else {
            continue;
        };
        let done = sqlx::query(
            "INSERT INTO match_results (match_id, owner_user_id, winner, score, finished_at) \
             VALUES (?, ?, ?, ?, ?) ON CONFLICT(match_id) DO NOTHING",
        )
        .bind(&m.id)
        .bind(owner)
        .bind(winner_text(winner))
        .bind(encode(&final_score)?)
        .bind(m.last_activity_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        if done.rows_affected() == 1 {
            inserted += 1;
        }
    }
    Ok(inserted)
}

/// The default and largest page of history.
pub const DEFAULT_PAGE: u32 = 30;
pub const MAX_PAGE: u32 = 100;

/// A player's results over every finished match they were picked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Record {
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

/// Where a history page ends: the next page holds the entries strictly older than this one, by
/// `(finished_at, match_id)`. Served as `"{finished_at}.{match_id}"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub finished_at: i64,
    pub match_id: String,
}

impl Cursor {
    /// `"{finished_at}.{match_id}"`, or `None` for anything else (no dot, a non-integer time or
    /// an empty id).
    pub fn parse(text: &str) -> Option<Cursor> {
        let (at, id) = text.split_once('.')?;
        let finished_at = at.parse().ok()?;
        if id.is_empty() {
            return None;
        }
        Some(Cursor {
            finished_at,
            match_id: id.to_owned(),
        })
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.finished_at, self.match_id)
    }
}

/// One finished match in the owner's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryEntry {
    pub match_id: String,
    pub finished_at: i64,
    /// The names as played (the match's snapshot, not today's player names).
    pub teams: Teams,
    /// The slots picked from the player list; `None` for a free-text slot.
    pub players: PlayerSlots,
    /// `None` for a timed draw.
    pub winner: Option<Team>,
    pub score: FinalScore,
    pub spectator_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Win,
    Loss,
    Draw,
}

/// A history entry seen from one player: the side they played on and how it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerHistoryEntry {
    #[serde(flatten)]
    pub entry: HistoryEntry,
    pub side: Team,
    pub outcome: Outcome,
}

/// A player's finishing place in an event (a tournament or a non-stop session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placing {
    pub event_id: String,
    pub event_kind: String,
    pub place: i64,
    /// Equal to `place` unless the place is shared (e.g. 3rd–4th).
    pub place_to: i64,
    pub field_size: i64,
    pub finished_at: i64,
}

fn bad_row(msg: String) -> StoreError {
    StoreError::Db(sqlx::Error::Decode(msg.into()))
}

fn parse_team(text: &str) -> Result<Team, StoreError> {
    match text {
        "a" => Ok(Team::A),
        "b" => Ok(Team::B),
        other => Err(bad_row(format!("unknown team {other:?}"))),
    }
}

fn json<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))
}

fn count(row: &SqliteRow, column: &str) -> Result<u32, StoreError> {
    let n: i64 = row.try_get(column)?;
    u32::try_from(n).map_err(|e| bad_row(e.to_string()))
}

/// The columns every history page selects, from `match_results r JOIN matches m`.
macro_rules! history_columns {
    () => {
        "r.match_id, r.finished_at, r.winner, r.score, m.teams, m.spectator_token"
    };
}

/// Append the keyset condition (entries strictly older than `before`), the newest-first order
/// and a `limit + 1` row limit, so the extra row tells whether there is a next page.
fn push_page(q: &mut QueryBuilder<Sqlite>, before: Option<&Cursor>, limit: u32) {
    if let Some(cursor) = before {
        q.push(" AND (r.finished_at, r.match_id) < (")
            .push_bind(cursor.finished_at)
            .push(", ")
            .push_bind(cursor.match_id.clone())
            .push(")");
    }
    q.push(" ORDER BY r.finished_at DESC, r.match_id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);
}

fn entry_from_row(row: &SqliteRow) -> Result<HistoryEntry, StoreError> {
    let winner: Option<String> = row.try_get("winner")?;
    Ok(HistoryEntry {
        match_id: row.try_get("match_id")?,
        finished_at: row.try_get("finished_at")?,
        teams: json(row.try_get::<&str, _>("teams")?)?,
        players: PlayerSlots::default(),
        winner: winner.as_deref().map(parse_team).transpose()?,
        score: json(row.try_get::<&str, _>("score")?)?,
        spectator_token: row.try_get("spectator_token")?,
    })
}

/// Keep `limit` rows and say where the next page starts, if there is one.
fn split_page<T>(
    mut items: Vec<T>,
    limit: u32,
    key: impl Fn(&T) -> &HistoryEntry,
) -> (Vec<T>, Option<Cursor>) {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    if items.len() <= limit {
        return (items, None);
    }
    items.truncate(limit);
    let next = items.last().map(|item| {
        let entry = key(item);
        Cursor {
            finished_at: entry.finished_at,
            match_id: entry.match_id.clone(),
        }
    });
    (items, next)
}

/// The record columns over `match_players mp JOIN match_results r`.
macro_rules! record_columns {
    () => {
        "SUM(CASE WHEN r.winner = mp.team THEN 1 ELSE 0 END) AS wins, \
         SUM(CASE WHEN r.winner <> mp.team THEN 1 ELSE 0 END) AS losses, \
         SUM(CASE WHEN r.winner IS NULL THEN 1 ELSE 0 END) AS draws"
    };
}

fn record_from_row(row: &SqliteRow) -> Result<Record, StoreError> {
    Ok(Record {
        wins: count(row, "wins")?,
        losses: count(row, "losses")?,
        draws: count(row, "draws")?,
    })
}

impl Store {
    /// Fill in each entry's picked players with one query over the page's match ids.
    async fn fill_players(&self, entries: &mut [&mut HistoryEntry]) -> Result<(), StoreError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut q = QueryBuilder::<Sqlite>::new(
            "SELECT match_id, team, position, player_id FROM match_players WHERE match_id IN (",
        );
        let mut ids = q.separated(", ");
        for entry in entries.iter() {
            ids.push_bind(entry.match_id.clone());
        }
        q.push(")");
        let rows = q.build().fetch_all(self.pool()).await?;
        let mut by_match: HashMap<&str, &mut PlayerSlots> = entries
            .iter_mut()
            .map(|e| (e.match_id.as_str(), &mut e.players))
            .collect();
        for row in &rows {
            let match_id: &str = row.try_get("match_id")?;
            let team = parse_team(row.try_get::<&str, _>("team")?)?;
            let position: i64 = row.try_get("position")?;
            let player_id: String = row.try_get("player_id")?;
            let Some(slots) = by_match.get_mut(match_id) else {
                continue;
            };
            let side = match team {
                Team::A => &mut slots.a,
                Team::B => &mut slots.b,
            };
            let slot = usize::try_from(position)
                .ok()
                .and_then(|p| side.get_mut(p))
                .ok_or_else(|| bad_row(format!("bad slot position {position}")))?;
            *slot = Some(player_id);
        }
        Ok(())
    }

    /// One page of the owner's finished matches, newest first, and where the next page starts.
    pub async fn history(
        &self,
        owner: &str,
        before: Option<&Cursor>,
        limit: u32,
    ) -> Result<(Vec<HistoryEntry>, Option<Cursor>), StoreError> {
        let limit = limit.max(1);
        let mut q = QueryBuilder::<Sqlite>::new(concat!(
            "SELECT ",
            history_columns!(),
            " FROM match_results r JOIN matches m ON m.id = r.match_id WHERE r.owner_user_id = "
        ));
        q.push_bind(owner.to_owned());
        push_page(&mut q, before, limit);
        let rows = q.build().fetch_all(self.pool()).await?;
        let items = rows
            .iter()
            .map(entry_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let (mut items, next) = split_page(items, limit, |e| e);
        self.fill_players(&mut items.iter_mut().collect::<Vec<_>>())
            .await?;
        Ok((items, next))
    }

    /// One page of the finished matches `player_id` was picked for, newest first; `None` when the
    /// player is not the owner's.
    pub async fn player_history(
        &self,
        owner: &str,
        player_id: &str,
        before: Option<&Cursor>,
        limit: u32,
    ) -> Result<Option<(Vec<PlayerHistoryEntry>, Option<Cursor>)>, StoreError> {
        if self.player(owner, player_id).await?.is_none() {
            return Ok(None);
        }
        let limit = limit.max(1);
        let mut q = QueryBuilder::<Sqlite>::new(concat!(
            "SELECT ",
            history_columns!(),
            ", mp.team AS side FROM match_players mp \
             JOIN match_results r ON r.match_id = mp.match_id \
             JOIN matches m ON m.id = r.match_id WHERE mp.player_id = "
        ));
        q.push_bind(player_id.to_owned())
            .push(" AND r.owner_user_id = ")
            .push_bind(owner.to_owned());
        push_page(&mut q, before, limit);
        let rows = q.build().fetch_all(self.pool()).await?;
        let items = rows
            .iter()
            .map(|row| {
                let entry = entry_from_row(row)?;
                let side = parse_team(row.try_get::<&str, _>("side")?)?;
                let outcome = match entry.winner {
                    None => Outcome::Draw,
                    Some(winner) if winner == side => Outcome::Win,
                    Some(_) => Outcome::Loss,
                };
                Ok(PlayerHistoryEntry {
                    entry,
                    side,
                    outcome,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let (mut items, next) = split_page(items, limit, |e| &e.entry);
        self.fill_players(&mut items.iter_mut().map(|e| &mut e.entry).collect::<Vec<_>>())
            .await?;
        Ok(Some((items, next)))
    }

    /// The record of every owner's player with at least one finished match, by player id. A
    /// player missing from the map has played none (`Record::default()`).
    pub async fn records(&self, owner: &str) -> Result<HashMap<String, Record>, StoreError> {
        let rows = sqlx::query(concat!(
            "SELECT p.id, ",
            record_columns!(),
            " FROM players p JOIN match_players mp ON mp.player_id = p.id \
             JOIN match_results r ON r.match_id = mp.match_id \
             WHERE p.owner_user_id = ? AND r.owner_user_id = p.owner_user_id GROUP BY p.id"
        ))
        .bind(owner)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| Ok((row.try_get("id")?, record_from_row(row)?)))
            .collect()
    }

    /// One player's record; zero for an unknown id or another owner's player.
    pub async fn record(&self, owner: &str, player_id: &str) -> Result<Record, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            record_columns!(),
            " FROM players p JOIN match_players mp ON mp.player_id = p.id \
             JOIN match_results r ON r.match_id = mp.match_id \
             WHERE p.owner_user_id = ? AND p.id = ? AND r.owner_user_id = p.owner_user_id \
             GROUP BY p.id"
        ))
        .bind(owner)
        .bind(player_id)
        .fetch_optional(self.pool())
        .await?;
        row.as_ref()
            .map(record_from_row)
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// The player's placings, newest first. Call only once the player is known to be the
    /// caller's: this does not check the owner.
    pub async fn placings(&self, player_id: &str) -> Result<Vec<Placing>, StoreError> {
        let rows = sqlx::query(
            "SELECT event_id, event_kind, place, place_to, field_size, finished_at FROM placings \
             WHERE player_id = ? ORDER BY finished_at DESC, event_id DESC",
        )
        .bind(player_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                Ok(Placing {
                    event_id: row.try_get("event_id")?,
                    event_kind: row.try_get("event_kind")?,
                    place: row.try_get("place")?,
                    place_to: row.try_get("place_to")?,
                    field_size: row.try_get("field_size")?,
                    finished_at: row.try_get("finished_at")?,
                })
            })
            .collect()
    }
}
