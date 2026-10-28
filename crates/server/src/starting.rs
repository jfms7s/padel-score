//! Starting a match from something that was planned: the hand-off between "Start match" on a
//! board game and the New match page.
//!
//! [`Store::start_game`] answers the page: the game's players (four, or two or three when short) as the starter's own player
//! entries, in seat order. [`create_match_from`] then makes the match and marks the source
//! played in the same transaction, so a source starts at most one match.

use padel_scoring::Format;
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use crate::boards::guests::is_guest_id;
use crate::linking::{auto_entry_in, plain_entry_in};
use crate::scheduled::{check_startable_in, mark_started_in};
use crate::store::{NewMatch, Store, StoreError, StoredMatch, Tx, create_match_in};

/// How long before its start a game can be started.
pub const START_WINDOW_SECS: i64 = 1800;

/// One of the four players of the match to start, as the starter's player entry.
#[derive(Debug, Clone, Serialize)]
pub struct StartPlayer {
    pub player_id: String,
    pub name: String,
}

/// What New match needs to fill in: four players in seat order (the first two are side A) and
/// the format when the source has one. A board game has none, so New match keeps its last
/// setup (`null`).
#[derive(Debug, Clone, Serialize)]
pub struct StartPlayers {
    pub players: Vec<StartPlayer>,
    pub format: Option<Format>,
    pub from: StartedFrom,
}

/// What a match was started from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartedFrom {
    Game { id: String },
    Scheduled { id: String },
}

/// A game's columns the start checks need.
struct StartGame {
    id: String,
    start: i64,
    end: i64,
    played: bool,
}

/// The window in which a game can be started: from [`START_WINDOW_SECS`] before its start until
/// its end.
pub(crate) fn check_window(start: i64, end: i64, now: i64) -> Result<(), StoreError> {
    if now < start - START_WINDOW_SECS {
        return Err(StoreError::Conflict("not_yet"));
    }
    if now >= end {
        return Err(StoreError::Conflict("game_over"));
    }
    Ok(())
}

/// The game's user ids in seat order. A user who is not among them is `NotFound`, like an
/// unknown game.
async fn seated_in(
    conn: &mut SqliteConnection,
    game_id: &str,
    user: &str,
) -> Result<Vec<String>, StoreError> {
    let seated: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM board_game_players WHERE game_id = ? ORDER BY seat",
    )
    .bind(game_id)
    .fetch_all(&mut *conn)
    .await?;
    if !seated.iter().any(|u| u == user) {
        return Err(StoreError::NotFound);
    }
    Ok(seated)
}

async fn game_in(conn: &mut SqliteConnection, game_id: &str) -> Result<StartGame, StoreError> {
    let row: Option<(i64, i64, bool)> = sqlx::query_as(
        "SELECT starts_at, ends_at, match_id IS NOT NULL FROM board_games WHERE id = ?",
    )
    .bind(game_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (start, end, played) = row.ok_or(StoreError::NotFound)?;
    Ok(StartGame {
        id: game_id.to_owned(),
        start,
        end,
        played,
    })
}

/// Create the match and mark what it came from as played, in the caller's transaction. The
/// match's players are not compared with the source's: the starter may sub.
///
/// For a scheduled match: the owner must be its owner (`NotFound` otherwise), it must not be
/// cancelled (`match_closed`) or started (`already_started`) and `now` must be in its window.
///
/// For a game: the owner must be one of its players (`NotFound` otherwise), it must not be
/// played (`already_started`) and `now` must be in its window (`not_yet`, `game_over`).
pub async fn create_match_from(
    store: &Store,
    new: NewMatch,
    from: &StartedFrom,
    now: i64,
) -> Result<StoredMatch, StoreError> {
    let mut tx = store.begin().await?;
    let owner = new.owner_user_id.clone().ok_or(StoreError::NotFound)?;
    let stored = match from {
        StartedFrom::Game { id } => {
            let game = game_in(&mut tx, id).await?;
            seated_in(&mut tx, &game.id, &owner).await?;
            if game.played {
                return Err(StoreError::Conflict("already_started"));
            }
            check_window(game.start, game.end, now)?;
            let stored = create_match_in(&mut tx, new, now).await?;
            mark_played_in(&mut tx, &game.id, &stored.id, now).await?;
            stored
        }
        StartedFrom::Scheduled { id } => {
            check_startable_in(&mut tx, id, &owner, now).await?;
            let stored = create_match_in(&mut tx, new, now).await?;
            mark_started_in(&mut tx, id, &stored.id, now).await?;
            stored
        }
    };
    tx.commit().await?;
    Ok(stored)
}

async fn mark_played_in(
    tx: &mut Tx<'_>,
    game_id: &str,
    match_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    let done = sqlx::query(
        "UPDATE board_games SET match_id = ?, played_at = ?, version = version + 1 \
         WHERE id = ? AND match_id IS NULL",
    )
    .bind(match_id)
    .bind(now)
    .bind(game_id)
    .execute(&mut **tx)
    .await?;
    if done.rows_affected() != 1 {
        return Err(StoreError::Conflict("already_started"));
    }
    Ok(())
}

impl Store {
    /// The two to four players of a board game for `user`, who must be one of them, as `user`'s own
    /// player entries (made, linked automatically, where there is none yet; a guest gets a plain
    /// unlinked entry). The board must not
    /// be over, the game not played and `now` in its start window.
    pub async fn start_game(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        now: i64,
    ) -> Result<StartPlayers, StoreError> {
        let mut tx = self.begin().await?;
        let board_end: i64 = sqlx::query_scalar("SELECT ends_at FROM boards WHERE token = ?")
            .bind(token)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound)?;
        let game = game_in_board_in(&mut tx, token, game_id).await?;
        let seated = seated_in(&mut tx, &game.id, user).await?;
        if board_end <= now {
            return Err(StoreError::Conflict("board_over"));
        }
        if game.played {
            return Err(StoreError::Conflict("already_started"));
        }
        check_window(game.start, game.end, now)?;
        // A short game can be started too (spec: Short -> Played): the starter fills the open
        // seats on New match.
        let mut players = Vec::with_capacity(seated.len());
        for who in &seated {
            let player_id = if is_guest_id(who) {
                // A guest has no account: the starter gets an unlinked entry under the guest's name.
                let guest_name: String =
                    sqlx::query_scalar("SELECT name FROM board_people WHERE id = ?")
                        .bind(who)
                        .fetch_one(&mut *tx)
                        .await?;
                plain_entry_in(&mut tx, user, &guest_name, now).await?
            } else {
                auto_entry_in(&mut tx, user, who, now).await?
            };
            let name: String = sqlx::query_scalar("SELECT name FROM players WHERE id = ?")
                .bind(&player_id)
                .fetch_one(&mut *tx)
                .await?;
            players.push(StartPlayer { player_id, name });
        }
        tx.commit().await?;
        Ok(StartPlayers {
            players,
            format: None,
            from: StartedFrom::Game { id: game.id },
        })
    }
}

/// [`game_in`] for a game that must be on the board with `token`.
async fn game_in_board_in(
    conn: &mut SqliteConnection,
    token: &str,
    game_id: &str,
) -> Result<StartGame, StoreError> {
    let on_board: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM board_games g JOIN boards b ON b.id = g.board_id \
         WHERE g.id = ? AND b.token = ?)",
    )
    .bind(game_id)
    .bind(token)
    .fetch_one(&mut *conn)
    .await?;
    if !on_board {
        return Err(StoreError::NotFound);
    }
    game_in(conn, game_id).await
}
