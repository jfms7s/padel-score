//! Linking a coordinator's player entry to an account. A link is one column,
//! `players.linked_user_id`; all history stays keyed by `players.id`, so linking and unlinking
//! move no rows. Every link path (link code, approved claim, link-me) goes through [`link_in`].

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{Row, SqliteConnection};

use crate::store::{Store, StoreError, Tx};

/// The account an entry is linked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub user_id: String,
    pub display_name: String,
    pub linked_at: i64,
}

/// One entry an account is linked to, as the account owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MyLink {
    pub player_id: String,
    pub name: String,
    pub coordinator_name: String,
    pub linked_at: i64,
}

/// All a claimant or link-code holder sees of an entry before it is linked: no scores,
/// opponents, partners or match links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimPreview {
    pub name: String,
    pub coordinator_name: String,
    pub matches: u32,
    pub events: u32,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
}

/// The entry's name, its coordinator's name and how often it appears. The one place that counts
/// appearances: finished matches (`match_players` joined to `match_results`) and events
/// (distinct `placings.event_id`); the dates are the earliest and latest `finished_at` of both.
/// An unknown entry is `NotFound`.
pub async fn preview_in(
    conn: &mut SqliteConnection,
    player_id: &str,
) -> Result<ClaimPreview, StoreError> {
    let head = sqlx::query(
        "SELECT p.name, COALESCE(u.display_name, '') AS coordinator_name FROM players p \
         LEFT JOIN users u ON u.id = p.owner_user_id WHERE p.id = ?",
    )
    .bind(player_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(StoreError::NotFound)?;
    let counts = sqlx::query(
        "SELECT \
           (SELECT COUNT(*) FROM match_players mp JOIN match_results r ON r.match_id = mp.match_id \
              WHERE mp.player_id = ?1) AS matches, \
           (SELECT COUNT(DISTINCT event_id) FROM placings WHERE player_id = ?1) AS events, \
           MIN(at) AS first_at, MAX(at) AS last_at \
         FROM (SELECT r.finished_at AS at FROM match_players mp \
                 JOIN match_results r ON r.match_id = mp.match_id WHERE mp.player_id = ?1 \
               UNION ALL SELECT finished_at FROM placings WHERE player_id = ?1)",
    )
    .bind(player_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(ClaimPreview {
        name: head.try_get("name")?,
        coordinator_name: head.try_get("coordinator_name")?,
        matches: counts.try_get::<i64, _>("matches")?.try_into().unwrap_or(0),
        events: counts.try_get::<i64, _>("events")?.try_into().unwrap_or(0),
        first_at: counts.try_get("first_at")?,
        last_at: counts.try_get("last_at")?,
    })
}

/// Link the entry to `user_id`. Linking it to the same user again is a no-op (the first link
/// time stays). Refused with `Conflict("player_linked")` when it is linked to someone else, and
/// with `Conflict("link_overlaps")` when it shares a match or an event with another entry
/// already linked to `user_id`: one person cannot be two sides of one game. An unknown entry is
/// `NotFound`.
pub async fn link_in(
    tx: &mut Tx<'_>,
    player_id: &str,
    user_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    let row = sqlx::query("SELECT linked_user_id FROM players WHERE id = ?")
        .bind(player_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    match row.try_get::<Option<String>, _>("linked_user_id")? {
        Some(current) if current == user_id => return Ok(()),
        Some(_) => return Err(StoreError::Conflict("player_linked")),
        None => {}
    }
    let overlaps: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
           SELECT 1 FROM match_players mine \
             JOIN match_players other ON other.match_id = mine.match_id \
              AND other.player_id <> mine.player_id \
             JOIN players q ON q.id = other.player_id \
           WHERE mine.player_id = ?1 AND q.linked_user_id = ?2 \
         ) OR EXISTS ( \
           SELECT 1 FROM placings mine \
             JOIN placings other ON other.event_id = mine.event_id \
              AND other.player_id <> mine.player_id \
             JOIN players q ON q.id = other.player_id \
           WHERE mine.player_id = ?1 AND q.linked_user_id = ?2 \
         )",
    )
    .bind(player_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    if overlaps {
        return Err(StoreError::Conflict("link_overlaps"));
    }
    sqlx::query("UPDATE players SET linked_user_id = ?, linked_at = ? WHERE id = ?")
        .bind(user_id)
        .bind(now)
        .bind(player_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// `NotFound` unless the entry belongs to `owner`.
async fn require_owned_in(
    conn: &mut SqliteConnection,
    owner: &str,
    player_id: &str,
) -> Result<(), StoreError> {
    sqlx::query("SELECT 1 FROM players WHERE id = ? AND owner_user_id = ?")
        .bind(player_id)
        .bind(owner)
        .fetch_optional(conn)
        .await?
        .map(|_| ())
        .ok_or(StoreError::NotFound)
}

impl Store {
    /// The coordinator links their own entry to their own account.
    pub async fn link_me(&self, owner: &str, player_id: &str, now: i64) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        require_owned_in(&mut tx, owner, player_id).await?;
        link_in(&mut tx, player_id, owner, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The coordinator unlinks one of their entries. Unlinking an unlinked entry is a no-op.
    pub async fn unlink(&self, owner: &str, player_id: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        require_owned_in(&mut tx, owner, player_id).await?;
        sqlx::query("UPDATE players SET linked_user_id = NULL, linked_at = NULL WHERE id = ?")
            .bind(player_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The linked account unlinks itself. `NotFound` unless the entry is linked to `user_id`.
    pub async fn unlink_self(&self, user_id: &str, player_id: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let done = sqlx::query(
            "UPDATE players SET linked_user_id = NULL, linked_at = NULL \
             WHERE id = ? AND linked_user_id = ?",
        )
        .bind(player_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    /// The owner's linked entries, by player id.
    pub async fn links(&self, owner: &str) -> Result<HashMap<String, Link>, StoreError> {
        let rows = sqlx::query(
            "SELECT p.id, p.linked_user_id, p.linked_at, u.display_name FROM players p \
             JOIN users u ON u.id = p.linked_user_id WHERE p.owner_user_id = ?",
        )
        .bind(owner)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("id")?,
                    Link {
                        user_id: row.try_get("linked_user_id")?,
                        display_name: row.try_get("display_name")?,
                        linked_at: row.try_get("linked_at")?,
                    },
                ))
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }

    /// The entries linked to `user_id`, newest link first.
    pub async fn my_links(&self, user_id: &str) -> Result<Vec<MyLink>, StoreError> {
        let rows = sqlx::query(
            "SELECT p.id, p.name, COALESCE(u.display_name, '') AS coordinator_name, p.linked_at \
             FROM players p LEFT JOIN users u ON u.id = p.owner_user_id \
             WHERE p.linked_user_id = ? ORDER BY p.linked_at DESC, p.id",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                Ok(MyLink {
                    player_id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    coordinator_name: row.try_get("coordinator_name")?,
                    linked_at: row.try_get("linked_at")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }
}
