//! The match results projection: one `match_results` row per finished owned match.
//!
//! The event log stays the source of truth. A row is derived from the engine's `Score` and written
//! only here: by [`sync_in`] inside each applied command's transaction, so an undo or replace that
//! un-finishes a match retracts its row, and by the startup [`backfill`]. Anonymous matches never
//! get a row. The winner is read from `Score.status`, never worked out here.

use padel_scoring::{Format, Score, SetScore, Status, Team};
use serde::{Deserialize, Serialize};

use crate::commands::view_of;
use crate::store::{Store, StoreError, StoredMatch, Tx, events_in, match_by_id_in};

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

fn winner_text(winner: Option<Team>) -> Option<&'static str> {
    winner.map(|team| match team {
        Team::A => "a",
        Team::B => "b",
    })
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
