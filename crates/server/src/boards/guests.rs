//! Guest players on a board: a participant id is a user id or a guest id.

use serde::Serialize;

use super::{BoardView, join_in, open_board_in, view_in};
use crate::players::{name_key, normalize_name};
use crate::store::{Store, StoreError};
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
#[derive(Debug, Serialize)]
pub struct GuestJoined {
    pub guest_key: String,
    pub view: BoardView,
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
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM board_guests WHERE id = ? AND board_id = ? AND gone_at IS NULL",
        )
        .bind(guest_id)
        .bind(&board.id)
        .fetch_one(&mut *tx)
        .await?;
        if live == 0 {
            return Err(StoreError::NotFound);
        }
        let guest_key = insert_key_in(&mut tx, guest_id, now).await?;
        let view = view_in(&mut tx, &board, Some(guest_id), now).await?;
        tx.commit().await?;
        Ok(GuestJoined { guest_key, view })
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
