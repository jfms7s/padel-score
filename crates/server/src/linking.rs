//! Linking a coordinator's player entry to an account. A link is one column,
//! `players.linked_user_id`; all history stays keyed by `players.id`, so linking and unlinking
//! move no rows. Every link path (link code, approved claim, link-me) goes through [`link_in`].

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{Row, SqliteConnection};

use crate::players::{name_key, normalize_name};
use crate::store::{MAX_NAME_CHARS, Store, StoreError, Tx};
use crate::tokens::{hash_token, new_token};

/// How long a link code works: 10 minutes.
pub const LINK_CODE_TTL_SECS: i64 = 600;

/// The account an entry is linked to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub user_id: String,
    pub display_name: String,
    pub linked_at: i64,
    /// Linked by a game, sign-up or scheduled match and not yet approved: privacy ignores it.
    pub automatic: bool,
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
/// appearances: finished matches (`match_players` joined to `match_results`) and events (the
/// distinct events of its `placings` and of the `competition_teams` that name it, so an event
/// still running counts too). The dates are the earliest and latest of the matches'
/// `finished_at`, the placings' `finished_at` and those events' `started_at`/`finished_at`.
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
           (SELECT COUNT(*) FROM ( \
              SELECT event_id FROM placings WHERE player_id = ?1 \
              UNION SELECT event_id FROM competition_teams \
                WHERE player1_id = ?1 OR player2_id = ?1)) AS events, \
           MIN(at) AS first_at, MAX(at) AS last_at \
         FROM (SELECT r.finished_at AS at FROM match_players mp \
                 JOIN match_results r ON r.match_id = mp.match_id WHERE mp.player_id = ?1 \
               UNION ALL SELECT finished_at FROM placings WHERE player_id = ?1 \
               UNION ALL SELECT c.started_at FROM competitions c \
                 JOIN competition_teams ct ON ct.event_id = c.id \
                 WHERE ct.player1_id = ?1 OR ct.player2_id = ?1 \
               UNION ALL SELECT c.finished_at FROM competitions c \
                 JOIN competition_teams ct ON ct.event_id = c.id \
                 WHERE ct.player1_id = ?1 OR ct.player2_id = ?1)",
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

/// Link the entry to `user_id`. Linking it to the same user again keeps the first link time but
/// still clears the automatic mark, so an approval, a link code or link-me makes the link count.
/// Refused with `Conflict("player_linked")` when it is linked to someone else, and with
/// `Conflict("link_overlaps")` when it shares a match or an event (a placing, or a team of the
/// same event) with another entry already linked to `user_id`: one person cannot be two sides of
/// one game. An unknown entry is `NotFound`.
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
        Some(current) if current == user_id => {
            sqlx::query("UPDATE players SET link_auto = 0 WHERE id = ?")
                .bind(player_id)
                .execute(&mut **tx)
                .await?;
            settle_own_claim(tx, player_id, user_id, now).await?;
            return Ok(());
        }
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
         ) OR EXISTS ( \
           SELECT 1 FROM competition_teams mine \
             JOIN competition_teams other ON other.event_id = mine.event_id \
             JOIN players q ON q.id IN (other.player1_id, other.player2_id) AND q.id <> ?1 \
           WHERE ?1 IN (mine.player1_id, mine.player2_id) AND q.linked_user_id = ?2 \
         )",
    )
    .bind(player_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    if overlaps {
        return Err(StoreError::Conflict("link_overlaps"));
    }
    sqlx::query("UPDATE players SET linked_user_id = ?, linked_at = ?, link_auto = 0 WHERE id = ?")
        .bind(user_id)
        .bind(now)
        .bind(player_id)
        .execute(&mut **tx)
        .await?;
    settle_own_claim(tx, player_id, user_id, now).await?;
    Ok(())
}

/// The linker's own pending claim on the entry is settled by the link. Other claimants' rows
/// stay pending: they reappear if the entry is unlinked.
async fn settle_own_claim(
    tx: &mut Tx<'_>,
    player_id: &str,
    user_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE player_claims SET status = 'approved', decided_at = ? \
         WHERE player_id = ? AND claimant_user_id = ? AND status = 'pending'",
    )
    .bind(now)
    .bind(player_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The highest number tried on a name clash.
const MAX_AUTO_NUMBER: u32 = 99;

/// `owner`'s entry for the account `user_id`, made if there is none, in the caller's
/// transaction:
///
/// - An entry of `owner` already linked to `user_id` is reused: the unarchived one, else the
///   newest archived one, un-archived. Its `link_auto` mark is left as it is.
/// - Otherwise a new entry is inserted, named after `users.display_name` (then "{name} 2" …
///   "{name} 99" on a `name_key` clash, the base cut so the result fits the name limit;
///   `Conflict("name_taken")` after that). It is linked at once with `link_auto = 1`, which
///   privacy ignores until a link code, a claim or link-me approves it. An owner's entry for
///   themself is not automatic.
pub async fn auto_entry_in(
    tx: &mut Tx<'_>,
    owner: &str,
    user_id: &str,
    now: i64,
) -> Result<String, StoreError> {
    let existing = sqlx::query(
        "SELECT id, archived_at FROM players WHERE owner_user_id = ? AND linked_user_id = ? \
         ORDER BY archived_at IS NOT NULL, created_at DESC, id LIMIT 1",
    )
    .bind(owner)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = existing {
        let id: String = row.try_get("id")?;
        if row.try_get::<Option<i64>, _>("archived_at")?.is_some() {
            sqlx::query("UPDATE players SET archived_at = NULL WHERE id = ?")
                .bind(&id)
                .execute(&mut **tx)
                .await?;
        }
        return Ok(id);
    }
    let display_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    let base = normalize_name(&display_name)?;
    for n in 1..=MAX_AUTO_NUMBER {
        let name = if n == 1 {
            base.clone()
        } else {
            let suffix = format!(" {n}");
            let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
            let cut: String = base.chars().take(room).collect();
            format!("{}{suffix}", cut.trim_end())
        };
        let key = name_key(&name);
        let clash: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM players WHERE owner_user_id = ? AND name_key = ?)",
        )
        .bind(owner)
        .bind(&key)
        .fetch_one(&mut **tx)
        .await?;
        if clash {
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO players (id, owner_user_id, name, name_key, archived_at, created_at, \
                                  linked_user_id, linked_at, link_auto) \
             VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(owner)
        .bind(&name)
        .bind(&key)
        .bind(now)
        .bind(user_id)
        .bind(now)
        .bind(i64::from(owner != user_id))
        .execute(&mut **tx)
        .await?;
        return Ok(id);
    }
    Err(StoreError::Conflict("name_taken"))
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
        sqlx::query(
            "UPDATE players SET linked_user_id = NULL, linked_at = NULL, link_auto = 0 \
             WHERE id = ?",
        )
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
            "UPDATE players SET linked_user_id = NULL, linked_at = NULL, link_auto = 0 \
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
            "SELECT p.id, p.linked_user_id, p.linked_at, p.link_auto, u.display_name FROM players p \
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
                        automatic: row.try_get("link_auto")?,
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

    /// A new link code for one of the owner's unlinked (or automatically linked) entries: the
    /// raw code (handed out once; only its hash is stored) and when it expires. The entry's
    /// other unused codes are deleted in the same transaction, so only the newest QR works.
    /// `NotFound` unless the entry is the owner's, `Conflict("player_linked")` when it is linked
    /// already (an automatic link is not).
    pub async fn create_link_code(
        &self,
        owner: &str,
        player_id: &str,
        now: i64,
    ) -> Result<(String, i64), StoreError> {
        let mut tx = self.begin().await?;
        require_owned_in(&mut tx, owner, player_id).await?;
        // An automatic link does not count as a link: its code is how the person approves it.
        let taken: bool = sqlx::query_scalar(
            "SELECT linked_user_id IS NOT NULL AND link_auto = 0 FROM players WHERE id = ?",
        )
        .bind(player_id)
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            return Err(StoreError::Conflict("player_linked"));
        }
        sqlx::query("DELETE FROM link_codes WHERE player_id = ? AND used_by IS NULL")
            .bind(player_id)
            .execute(&mut *tx)
            .await?;
        let raw = new_token();
        let expires_at = now + LINK_CODE_TTL_SECS;
        sqlx::query(
            "INSERT INTO link_codes (code_hash, player_id, created_by, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(hash_token(&raw))
        .bind(player_id)
        .bind(owner)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((raw, expires_at))
    }

    /// What the code holder sees of the entry. `None` for an unknown, expired or used code, or
    /// one whose entry was linked meanwhile: one answer for every reason.
    pub async fn link_code_preview(
        &self,
        raw: &str,
        now: i64,
    ) -> Result<Option<ClaimPreview>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let player_id: Option<String> = sqlx::query_scalar(
            "SELECT c.player_id FROM link_codes c JOIN players p ON p.id = c.player_id \
             WHERE c.code_hash = ? AND c.used_by IS NULL AND c.expires_at > ? \
               AND (p.linked_user_id IS NULL OR p.link_auto = 1)",
        )
        .bind(hash_token(raw))
        .bind(now)
        .fetch_optional(&mut *conn)
        .await?;
        match player_id {
            Some(id) => Ok(Some(preview_in(&mut conn, &id).await?)),
            None => Ok(None),
        }
    }

    /// Spend the code and link its entry to `user_id`, in one transaction: the player id, or
    /// `None` for a bad code (any reason). `link_in`'s conflicts pass through and roll the
    /// spend back, so the code stays unused.
    pub async fn accept_link_code(
        &self,
        raw: &str,
        user_id: &str,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        let mut tx = self.begin().await?;
        let player_id: Option<String> = sqlx::query_scalar(
            "UPDATE link_codes SET used_by = ?, used_at = ? \
             WHERE code_hash = ? AND used_by IS NULL AND expires_at > ? RETURNING player_id",
        )
        .bind(user_id)
        .bind(now)
        .bind(hash_token(raw))
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(player_id) = player_id else {
            return Ok(None);
        };
        link_in(&mut tx, &player_id, user_id, now).await?;
        tx.commit().await?;
        Ok(Some(player_id))
    }
}
