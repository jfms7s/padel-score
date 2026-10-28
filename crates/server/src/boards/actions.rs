//! What a player does to one game: leave it, give their place away, or take an open place. Every
//! action carries the game's `version` from the view the player saw, and a mismatch is
//! `game_changed`, so a stale sheet never moves anyone. Each is one `BEGIN IMMEDIATE`
//! transaction that ends in [`settle_in`].

use sqlx::SqliteConnection;

use super::formation::{self, Give, GiveRefusal, PLACES};
use super::ranges::{trim, uncovered_parts};
use super::{
    BoardRow, BoardView, MAX_PIECES_PER_MEMBER, display_name_in, find_by_id_in, find_by_token_in,
    insert_piece_in, join_in, load_state_in, mine, settle_in, view_in,
};
use crate::push::{Notice, NoticeKind, queue_in};
use crate::store::{Store, StoreError, Tx};

/// The columns of a `board_games` row the actions need.
struct GameRow {
    start: i64,
    end: i64,
    version: i64,
    played: bool,
}

async fn game_row_in(
    conn: &mut SqliteConnection,
    board_id: &str,
    game_id: &str,
) -> Result<GameRow, StoreError> {
    let row: Option<(i64, i64, i64, bool)> = sqlx::query_as(
        "SELECT starts_at, ends_at, version, match_id IS NOT NULL FROM board_games \
         WHERE id = ? AND board_id = ?",
    )
    .bind(game_id)
    .bind(board_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (start, end, version, played) = row.ok_or(StoreError::NotFound)?;
    Ok(GameRow {
        start,
        end,
        version,
        played,
    })
}

/// The board and game for an action, checked in this order: the token and the game on that
/// board are `NotFound`, then `board_over`, then a stale `version` is `game_changed`.
async fn open_game_in(
    conn: &mut SqliteConnection,
    token: &str,
    game_id: &str,
    version: i64,
    now: i64,
) -> Result<(BoardRow, GameRow), StoreError> {
    let board = find_by_token_in(conn, token)
        .await?
        .ok_or(StoreError::NotFound)?;
    let game = game_row_in(conn, &board.id, game_id).await?;
    if board.over(now) {
        return Err(StoreError::Conflict("board_over"));
    }
    if game.version != version {
        return Err(StoreError::Conflict("game_changed"));
    }
    Ok((board, game))
}

/// A played game, or one that has started, takes no more actions.
fn still_open(game: &GameRow, now: i64) -> Result<(), StoreError> {
    if game.played {
        return Err(StoreError::Conflict("game_played"));
    }
    if game.start <= now {
        return Err(StoreError::Conflict("game_started"));
    }
    Ok(())
}

async fn bump_version_in(tx: &mut Tx<'_>, game_id: &str) -> Result<(), StoreError> {
    sqlx::query("UPDATE board_games SET version = version + 1 WHERE id = ?")
        .bind(game_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn drop_line_head_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM board_line_heads WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn seat_of_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<i64, StoreError> {
    let seat: Option<i64> =
        sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ? AND user_id = ?")
            .bind(game_id)
            .bind(user)
            .fetch_optional(&mut **tx)
            .await?;
    seat.ok_or_else(|| StoreError::Corrupt(format!("{user} has no seat in game {game_id}")))
}

async fn seat_in(
    tx: &mut Tx<'_>,
    game_id: &str,
    user: &str,
    seat: i64,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) VALUES (?, ?, ?, ?)",
    )
    .bind(game_id)
    .bind(user)
    .bind(seat)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn unseat_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM board_game_players WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Take `user` out of a game. An emptied game is deleted; otherwise its version goes up and the
/// players left are told. The leaver stops heading the game's line. It does not fill the seat:
/// [`settle_in`] does, after the caller's changes are all made.
pub(crate) async fn leave_in(
    tx: &mut Tx<'_>,
    board_id: &str,
    game_id: &str,
    user: &str,
    now: i64,
) -> Result<(), StoreError> {
    drop_line_head_in(tx, game_id, user).await?;
    let removed = sqlx::query("DELETE FROM board_game_players WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    if removed.rows_affected() == 0 {
        return Ok(());
    }
    let rest: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM board_game_players WHERE game_id = ? ORDER BY seat",
    )
    .bind(game_id)
    .fetch_all(&mut **tx)
    .await?;
    if rest.is_empty() {
        sqlx::query("DELETE FROM board_games WHERE id = ?")
            .bind(game_id)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }
    bump_version_in(tx, game_id).await?;
    let game = game_row_in(tx, board_id, game_id).await?;
    let board = find_by_id_in(tx, board_id).await?;
    let name = display_name_in(tx, user).await?;
    let notices: Vec<Notice> = rest
        .iter()
        .map(|other| {
            board.notice(
                other,
                NoticeKind::PlayerLeft,
                board.board_url(),
                (game.start, game.end),
                Some(name.clone()),
            )
        })
        .collect();
    queue_in(tx, &notices, now).await?;
    Ok(())
}

impl Store {
    /// Leave a game. The leaver's free time over the game goes too, or the next settle would
    /// seat them straight back. The first in the line takes the place, unless the game starts
    /// inside the fill cutoff: then it stays short.
    pub async fn leave_game(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let seated = state
            .games
            .iter()
            .any(|g| g.id == game_id && g.players.iter().any(|p| p == user));
        if !seated {
            return Err(StoreError::Conflict("not_in_game"));
        }
        let existing = mine(&state, user);
        let cut = trim(&existing, game.start, game.end);
        if existing.len().saturating_sub(cut.delete.len()) + cut.insert.len()
            > MAX_PIECES_PER_MEMBER
        {
            return Err(StoreError::Conflict("too_many_ranges"));
        }
        for id in &cut.delete {
            sqlx::query("DELETE FROM board_ranges WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for (start, end, created_at) in cut.insert {
            insert_piece_in(&mut tx, &board.id, user, start, end, created_at).await?;
        }
        leave_in(&mut tx, &board.id, game_id, user, now).await?;
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Give the player's place away: to the next overlapping game, trading seats with its best
    /// free player, or else to the front of the line. See [`formation::give_place`].
    pub async fn give_place(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let give = formation::give_place(&state, game_id, user, now).map_err(|refusal| {
            StoreError::Conflict(match refusal {
                GiveRefusal::NotInGame => "not_in_game",
                GiveRefusal::TooLate => "too_late",
                GiveRefusal::NobodyCanTake => "nobody_can_take",
            })
        })?;
        let seat = seat_of_in(&mut tx, game_id, user).await?;
        let name = display_name_in(&mut tx, user).await?;
        match give {
            Give::Trade {
                other_game,
                other_user,
            } => {
                let other = game_row_in(&mut tx, &board.id, &other_game).await?;
                let other_seat = seat_of_in(&mut tx, &other_game, &other_user).await?;
                unseat_in(&mut tx, game_id, user).await?;
                unseat_in(&mut tx, &other_game, &other_user).await?;
                seat_in(&mut tx, &other_game, user, other_seat, now).await?;
                seat_in(&mut tx, game_id, &other_user, seat, now).await?;
                for (g, u) in [
                    (game_id, user),
                    (game_id, other_user.as_str()),
                    (other_game.as_str(), user),
                    (other_game.as_str(), other_user.as_str()),
                ] {
                    drop_line_head_in(&mut tx, g, u).await?;
                }
                bump_version_in(&mut tx, game_id).await?;
                bump_version_in(&mut tx, &other_game).await?;
                let mut notice = board.notice(
                    &other_user,
                    NoticeKind::PlaceGiven,
                    board.board_url(),
                    (other.start, other.end),
                    Some(name),
                );
                notice.payload.new_start = Some(game.start);
                notice.payload.new_end = Some(game.end);
                queue_in(&mut tx, &[notice], now).await?;
            }
            Give::ToLine { replacement } => {
                unseat_in(&mut tx, game_id, user).await?;
                seat_in(&mut tx, game_id, &replacement, seat, now).await?;
                drop_line_head_in(&mut tx, game_id, &replacement).await?;
                sqlx::query(
                    "INSERT OR REPLACE INTO board_line_heads (game_id, user_id, at) \
                     VALUES (?, ?, ?)",
                )
                .bind(game_id)
                .bind(user)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                bump_version_in(&mut tx, game_id).await?;
                let notice = board.notice(
                    &replacement,
                    NoticeKind::MovedIn,
                    board.board_url(),
                    (game.start, game.end),
                    None,
                );
                queue_in(&mut tx, &[notice], now).await?;
            }
        }
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Take a short game's open place by hand, whatever the fill cutoff. It joins the board if
    /// needed and adds the taker's free time over the game, so the next settle keeps them in.
    pub async fn take_place(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let seated: Vec<String> = state
            .games
            .iter()
            .find(|g| g.id == game_id)
            .map(|g| g.players.clone())
            .ok_or(StoreError::NotFound)?;
        if seated.iter().any(|p| p == user) {
            return Err(StoreError::Conflict("already_in_game"));
        }
        if seated.len() >= PLACES {
            return Err(StoreError::Conflict("game_full"));
        }
        if state.busy(user, game.start, game.end, None) {
            return Err(StoreError::Conflict("busy"));
        }
        join_in(&mut tx, &board.id, user, now).await?;
        let existing = mine(&state, user);
        let parts = uncovered_parts(&existing, game.start, game.end);
        if existing.len() + parts.len() > MAX_PIECES_PER_MEMBER {
            return Err(StoreError::Conflict("too_many_ranges"));
        }
        for (start, end) in parts {
            insert_piece_in(&mut tx, &board.id, user, start, end, now).await?;
        }
        drop_line_head_in(&mut tx, game_id, user).await?;
        let taken: Vec<i64> =
            sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ?")
                .bind(game_id)
                .fetch_all(&mut *tx)
                .await?;
        let seat = (0_i64..)
            .take(PLACES)
            .find(|s| !taken.contains(s))
            .ok_or(StoreError::Conflict("game_full"))?;
        seat_in(&mut tx, game_id, user, seat, now).await?;
        bump_version_in(&mut tx, game_id).await?;
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }
}
