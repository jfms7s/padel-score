//! Guest players on a board: a participant id is a user id or a guest id.

use serde::Serialize;

use super::ranges::{Piece, uncovered_parts};
use super::{
    BoardView, MAX_PIECES_PER_MEMBER, bump_version_in, drop_line_head_in, insert_piece_in, join_in,
    leave_board_in, leave_in, load_state_in, mine, open_board_in, settle_in, view_in,
};
use crate::players::{name_key, normalize_name};
use crate::store::{Store, StoreError, Tx};
use crate::tokens::{hash_token, new_token};

/// Guest ids start with this; user ids are bare UUIDs, so the two never clash.
pub const GUEST_ID_PREFIX: &str = "guest_";

/// Whether a board participant id belongs to a guest.
pub fn is_guest_id(id: &str) -> bool {
    id.starts_with(GUEST_ID_PREFIX)
}

/// A fresh guest id: the prefix and a v4 UUID.
pub(crate) fn new_guest_id() -> String {
    format!("{GUEST_ID_PREFIX}{}", uuid::Uuid::new_v4())
}

/// A guest who just joined or was picked again: the secret to keep, and the board as they see it.
#[derive(Serialize)]
pub struct GuestJoined {
    pub guest_key: String,
    pub view: BoardView,
}

// The key is a secret: a stray `{:?}` must not print it.
impl std::fmt::Debug for GuestJoined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestJoined")
            .field("guest_key", &"<redacted>")
            .field("view", &self.view)
            .finish()
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

impl Store {
    /// Join the board as a new guest with this name. The guest is a member at once.
    pub async fn join_as_guest(
        &self,
        token: &str,
        name: &str,
        now: i64,
    ) -> Result<GuestJoined, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if !board.allow_guests {
            return Err(StoreError::Conflict("guests_off"));
        }
        let name = normalize_name(name).map_err(|_| StoreError::InvalidField("name"))?;
        let key = name_key(&name);
        let clash: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_guests \
             WHERE board_id = ? AND name_key = ? AND gone_at IS NULL",
        )
        .bind(&board.id)
        .bind(&key)
        .fetch_one(&mut *tx)
        .await?;
        if clash > 0 {
            return Err(StoreError::Conflict("guest_name_taken"));
        }
        let guest_id = new_guest_id();
        sqlx::query(
            "INSERT INTO board_guests (id, board_id, name, name_key, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&guest_id)
        .bind(&board.id)
        .bind(&name)
        .bind(&key)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if is_unique_violation(&e) {
                StoreError::Conflict("guest_name_taken")
            } else {
                e.into()
            }
        })?;
        join_in(&mut tx, &board.id, &guest_id, now).await?;
        let guest_key = insert_key_in(&mut tx, &guest_id, now).await?;
        let view = view_in(&mut tx, &board, Some(&guest_id), now).await?;
        tx.commit().await?;
        Ok(GuestJoined { guest_key, view })
    }

    /// Act as an existing live guest of the board from a new device: a second key for them.
    pub async fn pick_guest(
        &self,
        token: &str,
        guest_id: &str,
        now: i64,
    ) -> Result<GuestJoined, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if !board.allow_guests {
            return Err(StoreError::Conflict("guests_off"));
        }
        if !is_live_guest_in(&mut tx, &board.id, guest_id).await? {
            return Err(StoreError::NotFound);
        }
        let guest_key = insert_key_in(&mut tx, guest_id, now).await?;
        let view = view_in(&mut tx, &board, Some(guest_id), now).await?;
        tx.commit().await?;
        Ok(GuestJoined { guest_key, view })
    }

    /// Remove a live guest from the board. Creator only, and still allowed with guests off. The guest
    /// leaves like a member would: played and ended games keep their seat as history.
    pub async fn remove_guest(
        &self,
        token: &str,
        creator: &str,
        guest_id: &str,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if board.creator_user_id != creator {
            return Err(StoreError::Conflict("creator_only"));
        }
        if !is_live_guest_in(&mut tx, &board.id, guest_id).await? {
            return Err(StoreError::NotFound);
        }
        leave_board_in(&mut tx, &board, guest_id, now).await?;
        end_guest_in(&mut tx, guest_id, now).await?;
        let view = view_in(&mut tx, &board, Some(creator), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// An account takes over a live guest of the board: their free time, and their seats in games
    /// that have not been played. A seat the account cannot take (it is already in that game, or
    /// plays an overlapping one) is freed for the line instead. Played and ended games keep the
    /// guest's name as history.
    pub async fn claim_guest(
        &self,
        token: &str,
        user: &str,
        guest_id: &str,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if is_guest_id(user) || !is_live_guest_in(&mut tx, &board.id, guest_id).await? {
            return Err(StoreError::NotFound);
        }
        let state = load_state_in(&mut tx, &board.id).await?;
        let kept = mine(&state, user);
        let added = claimed_pieces(&kept, &mine(&state, guest_id), user);
        if kept.len() + added.len() > MAX_PIECES_PER_MEMBER {
            return Err(StoreError::Conflict("too_many_ranges"));
        }

        let upcoming = state
            .games
            .iter()
            .filter(|g| !g.played && g.end > now && g.players.iter().any(|p| p == guest_id));
        for game in upcoming {
            let claimer_in = game.players.iter().any(|p| p == user);
            if claimer_in || state.busy(user, game.start, game.end, Some(&game.id)) {
                leave_in(&mut tx, &board.id, &game.id, guest_id, now).await?;
            } else {
                sqlx::query(
                    "UPDATE board_game_players SET user_id = ? WHERE game_id = ? AND user_id = ?",
                )
                .bind(user)
                .bind(&game.id)
                .bind(guest_id)
                .execute(&mut *tx)
                .await?;
                bump_version_in(&mut tx, &game.id).await?;
                drop_line_head_in(&mut tx, &game.id, user).await?;
            }
        }
        move_line_heads_in(&mut tx, guest_id, user).await?;
        sqlx::query(
            "UPDATE board_requests SET status = 'expired', decided_at = ? \
             WHERE board_id = ? AND status = 'pending' AND (from_user_id = ? OR to_user_id = ?)",
        )
        .bind(now)
        .bind(&board.id)
        .bind(guest_id)
        .bind(guest_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM board_ranges WHERE board_id = ? AND user_id = ?")
            .bind(&board.id)
            .bind(guest_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM board_members WHERE board_id = ? AND user_id = ?")
            .bind(&board.id)
            .bind(guest_id)
            .execute(&mut *tx)
            .await?;
        end_guest_in(&mut tx, guest_id, now).await?;

        join_in(&mut tx, &board.id, user, now).await?;
        for (start, end, created_at) in added {
            insert_piece_in(&mut tx, &board.id, user, start, end, created_at).await?;
        }
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// The guest id a raw key stands for, if that guest is live and on the board with `token`.
    pub async fn guest_for_key(
        &self,
        token: &str,
        raw_key: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT g.id FROM board_guest_keys k \
             JOIN board_guests g ON g.id = k.guest_id \
             JOIN boards b ON b.id = g.board_id \
             WHERE k.key_hash = ? AND b.token = ? AND g.gone_at IS NULL",
        )
        .bind(hash_token(raw_key))
        .bind(token)
        .fetch_optional(self.pool())
        .await?)
    }
}

/// Mint a key for the guest, store its hash and return the raw value.
async fn insert_key_in(
    conn: &mut sqlx::SqliteConnection,
    guest_id: &str,
    now: i64,
) -> Result<String, StoreError> {
    let raw = new_token();
    sqlx::query("INSERT INTO board_guest_keys (key_hash, guest_id, created_at) VALUES (?, ?, ?)")
        .bind(hash_token(&raw))
        .bind(guest_id)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    Ok(raw)
}

async fn is_live_guest_in(
    conn: &mut sqlx::SqliteConnection,
    board_id: &str,
    guest_id: &str,
) -> Result<bool, StoreError> {
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM board_guests WHERE id = ? AND board_id = ? AND gone_at IS NULL",
    )
    .bind(guest_id)
    .bind(board_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(live > 0)
}

/// The guest is gone: their name is free again and their keys stop working. The row stays, for
/// the seats they held in played and ended games.
pub(super) async fn end_guest_in(
    tx: &mut Tx<'_>,
    guest_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE board_guests SET gone_at = ? WHERE id = ? AND gone_at IS NULL")
        .bind(now)
        .bind(guest_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM board_guest_keys WHERE guest_id = ?")
        .bind(guest_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The pieces the claimer gains: the parts of the guest's free time they do not already have,
/// each keeping the guest's `created_at` so it keeps its place in line.
fn claimed_pieces(kept: &[Piece], guest: &[Piece], claimer: &str) -> Vec<(i64, i64, i64)> {
    let mut covered = kept.to_vec();
    let mut added = Vec::new();
    let mut ordered: Vec<&Piece> = guest.iter().collect();
    ordered.sort_by_key(|p| (p.start, p.end));
    for piece in ordered {
        for (start, end) in uncovered_parts(&covered, piece.start, piece.end) {
            covered.push(Piece {
                id: String::new(),
                user_id: claimer.to_owned(),
                start,
                end,
                created_at: piece.created_at,
            });
            added.push((start, end, piece.created_at));
        }
    }
    added
}

/// The guest's places in game lines go to the claimer. Where the claimer already has one, the
/// earlier of the two stays; where the claimer already has the game's seat, the line place is moot.
async fn move_line_heads_in(
    tx: &mut Tx<'_>,
    guest_id: &str,
    claimer: &str,
) -> Result<(), StoreError> {
    let heads: Vec<(String, i64)> =
        sqlx::query_as("SELECT game_id, at FROM board_line_heads WHERE user_id = ?")
            .bind(guest_id)
            .fetch_all(&mut **tx)
            .await?;
    for (game_id, at) in heads {
        sqlx::query("DELETE FROM board_line_heads WHERE game_id = ? AND user_id = ?")
            .bind(&game_id)
            .bind(guest_id)
            .execute(&mut **tx)
            .await?;
        let seated: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_game_players WHERE game_id = ? AND user_id = ?",
        )
        .bind(&game_id)
        .bind(claimer)
        .fetch_one(&mut **tx)
        .await?;
        if seated > 0 {
            continue;
        }
        sqlx::query(
            "INSERT INTO board_line_heads (game_id, user_id, at) VALUES (?, ?, ?) \
             ON CONFLICT (game_id, user_id) DO UPDATE SET at = MIN(at, excluded.at)",
        )
        .bind(&game_id)
        .bind(claimer)
        .bind(at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
