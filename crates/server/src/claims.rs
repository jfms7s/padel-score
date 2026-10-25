//! Claims: a signed-in player asks to take over a coordinator's entry ("That's me" on a match
//! they played in), and the coordinator approves or declines. Before approval the claimant sees
//! only a [`ClaimPreview`]. Every link goes through [`link_in`].

use std::collections::HashMap;

use padel_scoring::Team;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use crate::linking::{ClaimPreview, link_in, preview_in};
use crate::store::{Store, StoreError};

/// How many claims one person can have waiting at once.
pub const MAX_PENDING_CLAIMS: i64 = 20;

/// What a claim points at. Seam point: events add a variant for a team slot of an event.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaimTarget {
    MatchSlot {
        spectator_token: String,
        team: Team,
        position: u8,
    },
}

fn team_str(team: Team) -> &'static str {
    match team {
        Team::A => "a",
        Team::B => "b",
    }
}

/// The entry a target points at and its coordinator: `(player_id, owner_user_id)`. `None` for an
/// unknown token, an anonymous match, a free-text slot or a slot that does not exist.
pub(crate) async fn resolve_target_in(
    conn: &mut SqliteConnection,
    target: &ClaimTarget,
) -> Result<Option<(String, String)>, StoreError> {
    let ClaimTarget::MatchSlot {
        spectator_token,
        team,
        position,
    } = target;
    let row = sqlx::query(
        "SELECT p.id, p.owner_user_id FROM matches m \
         JOIN match_players mp ON mp.match_id = m.id \
         JOIN players p ON p.id = mp.player_id \
         WHERE m.spectator_token = ? AND m.owner_user_id IS NOT NULL \
           AND mp.team = ? AND mp.position = ?",
    )
    .bind(spectator_token)
    .bind(team_str(*team))
    .bind(i64::from(*position))
    .fetch_optional(conn)
    .await?;
    row.map(|r| Ok((r.try_get("id")?, r.try_get("owner_user_id")?)))
        .transpose()
}

/// Where a picked slot stands for the viewing account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Claimable,
    Yours,
    Pending,
    Linked,
    Declined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlotState {
    pub team: Team,
    pub position: u8,
    pub name: String,
    pub state: SlotKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Pending,
    Approved,
    Declined,
    Withdrawn,
}

impl ClaimStatus {
    fn parse(text: &str) -> Result<ClaimStatus, StoreError> {
        match text {
            "pending" => Ok(ClaimStatus::Pending),
            "approved" => Ok(ClaimStatus::Approved),
            "declined" => Ok(ClaimStatus::Declined),
            "withdrawn" => Ok(ClaimStatus::Withdrawn),
            _ => Err(StoreError::Conflict("claim_status")),
        }
    }
}

/// A claim as its claimant sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MyClaim {
    pub id: String,
    pub status: ClaimStatus,
    pub preview: ClaimPreview,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimPlayer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Claimant {
    pub display_name: String,
    pub email: String,
}

/// A pending claim as the entry's coordinator sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Claim {
    pub id: String,
    pub player: ClaimPlayer,
    pub claimant: Claimant,
    pub preview: ClaimPreview,
    pub created_at: i64,
}

/// A pending claim about to be decided; `NotFound` unless `who` is its owner or claimant,
/// `Conflict("claim_decided")` unless it is pending.
struct Pending {
    player_id: String,
    claimant: String,
}

#[derive(Clone, Copy)]
enum Side {
    Owner,
    Claimant,
}

async fn pending_claim_in(
    conn: &mut SqliteConnection,
    side: Side,
    who: &str,
    claim_id: &str,
) -> Result<Pending, StoreError> {
    let sql = match side {
        Side::Owner => {
            "SELECT player_id, claimant_user_id, status FROM player_claims \
             WHERE id = ? AND owner_user_id = ?"
        }
        Side::Claimant => {
            "SELECT player_id, claimant_user_id, status FROM player_claims \
             WHERE id = ? AND claimant_user_id = ?"
        }
    };
    let row = sqlx::query(sql)
        .bind(claim_id)
        .bind(who)
        .fetch_optional(conn)
        .await?
        .ok_or(StoreError::NotFound)?;
    let status: String = row.try_get("status")?;
    if status != "pending" {
        return Err(StoreError::Conflict("claim_decided"));
    }
    Ok(Pending {
        player_id: row.try_get("player_id")?,
        claimant: row.try_get("claimant_user_id")?,
    })
}

async fn my_claim_in(conn: &mut SqliteConnection, id: &str) -> Result<MyClaim, StoreError> {
    let row = sqlx::query(
        "SELECT id, player_id, status, created_at, decided_at FROM player_claims WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    my_claim_of(conn, &row).await
}

async fn my_claim_of(
    conn: &mut SqliteConnection,
    row: &sqlx::sqlite::SqliteRow,
) -> Result<MyClaim, StoreError> {
    let player_id: String = row.try_get("player_id")?;
    Ok(MyClaim {
        id: row.try_get("id")?,
        status: ClaimStatus::parse(&row.try_get::<String, _>("status")?)?,
        preview: preview_in(conn, &player_id).await?,
        created_at: row.try_get("created_at")?,
        decided_at: row.try_get("decided_at")?,
    })
}

impl Store {
    /// One state per picked slot of the match, for the signed-in viewer. `None` for an unknown
    /// token; an anonymous match has no picked slots. A free-text slot is not listed.
    pub async fn slots(
        &self,
        spectator_token: &str,
        user_id: &str,
    ) -> Result<Option<Vec<SlotState>>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let found: Option<String> =
            sqlx::query_scalar("SELECT id FROM matches WHERE spectator_token = ?")
                .bind(spectator_token)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(match_id) = found else {
            return Ok(None);
        };
        let rows = sqlx::query(
            "SELECT mp.team, mp.position, p.name, p.linked_user_id, c.status AS claim \
             FROM match_players mp JOIN players p ON p.id = mp.player_id \
             LEFT JOIN player_claims c ON c.player_id = p.id AND c.claimant_user_id = ?1 \
             WHERE mp.match_id = ?2 ORDER BY mp.team, mp.position",
        )
        .bind(user_id)
        .bind(match_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let linked: Option<String> = row.try_get("linked_user_id")?;
            let claim: Option<String> = row.try_get("claim")?;
            let state = match (linked.as_deref(), claim.as_deref()) {
                (Some(u), _) if u == user_id => SlotKind::Yours,
                (Some(_), _) => SlotKind::Linked,
                (None, Some("pending")) => SlotKind::Pending,
                (None, Some("declined")) => SlotKind::Declined,
                _ => SlotKind::Claimable,
            };
            let team = match row.try_get::<String, _>("team")?.as_str() {
                "a" => Team::A,
                _ => Team::B,
            };
            out.push(SlotState {
                team,
                position: u8::try_from(row.try_get::<i64, _>("position")?).unwrap_or(0),
                name: row.try_get("name")?,
                state,
            });
        }
        Ok(Some(out))
    }

    /// What a claimant sees of the target's entry. `None` when the target does not resolve.
    pub async fn claim_preview(
        &self,
        target: &ClaimTarget,
    ) -> Result<Option<ClaimPreview>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        match resolve_target_in(&mut conn, target).await? {
            Some((player_id, _)) => Ok(Some(preview_in(&mut conn, &player_id).await?)),
            None => Ok(None),
        }
    }

    /// Ask to take over the target's entry: the claim, and whether this call created (or
    /// re-opened) it, which `linking_api` maps to 201 or 200. The coordinator's own claim links at once.
    pub async fn create_claim(
        &self,
        target: &ClaimTarget,
        user_id: &str,
        now: i64,
    ) -> Result<(MyClaim, bool), StoreError> {
        let mut tx = self.begin().await?;
        let (player_id, owner) = resolve_target_in(&mut tx, target)
            .await?
            .ok_or(StoreError::NotFound)?;
        let linked: Option<String> =
            sqlx::query_scalar("SELECT linked_user_id FROM players WHERE id = ?")
                .bind(&player_id)
                .fetch_one(&mut *tx)
                .await?;
        match linked {
            Some(u) if u == user_id => return Err(StoreError::Conflict("already_yours")),
            Some(_) => return Err(StoreError::Conflict("player_linked")),
            None => {}
        }
        let existing = sqlx::query(
            "SELECT id, status FROM player_claims WHERE player_id = ? AND claimant_user_id = ?",
        )
        .bind(&player_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let existing_id: Option<String> = existing.as_ref().map(|r| r.get("id"));
        let existing_status: Option<String> = existing.as_ref().map(|r| r.get("status"));

        let (id, created) = if owner == user_id {
            link_in(&mut tx, &player_id, user_id, now).await?;
            let id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            upsert(
                &mut tx,
                &id,
                &player_id,
                user_id,
                &owner,
                "approved",
                now,
                Some(now),
            )
            .await?;
            (id, false)
        } else {
            match (existing_id, existing_status.as_deref()) {
                (Some(_), Some("declined")) => return Err(StoreError::Conflict("claim_declined")),
                (Some(id), Some("pending")) => (id, false),
                (existing_id, _) => {
                    let pending: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM player_claims \
                         WHERE claimant_user_id = ? AND status = 'pending'",
                    )
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if pending >= MAX_PENDING_CLAIMS {
                        return Err(StoreError::Conflict("too_many_claims"));
                    }
                    let id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    upsert(
                        &mut tx, &id, &player_id, user_id, &owner, "pending", now, None,
                    )
                    .await?;
                    (id, true)
                }
            }
        };
        let claim = my_claim_in(&mut tx, &id).await?;
        tx.commit().await?;
        Ok((claim, created))
    }

    /// Pending claims on the owner's entries, oldest first, without disabled claimants.
    pub async fn incoming_claims(&self, owner: &str) -> Result<Vec<Claim>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let rows = sqlx::query(
            "SELECT c.id, c.player_id, c.created_at, p.name, u.display_name, u.email \
             FROM player_claims c JOIN players p ON p.id = c.player_id \
             JOIN users u ON u.id = c.claimant_user_id \
             WHERE c.owner_user_id = ? AND c.status = 'pending' AND u.disabled_at IS NULL \
             ORDER BY c.created_at, c.id",
        )
        .bind(owner)
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let player_id: String = row.try_get("player_id")?;
            out.push(Claim {
                id: row.try_get("id")?,
                player: ClaimPlayer {
                    id: player_id.clone(),
                    name: row.try_get("name")?,
                },
                claimant: Claimant {
                    display_name: row.try_get("display_name")?,
                    email: row.try_get("email")?,
                },
                preview: preview_in(&mut conn, &player_id).await?,
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(out)
    }

    /// The coordinator approves: link the entry, mark the claim approved and the entry's other
    /// pending claims declined. A link conflict leaves the claim pending.
    pub async fn approve_claim(
        &self,
        owner: &str,
        claim_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let claim = pending_claim_in(&mut tx, Side::Owner, owner, claim_id).await?;
        link_in(&mut tx, &claim.player_id, &claim.claimant, now).await?;
        sqlx::query("UPDATE player_claims SET status = 'approved', decided_at = ? WHERE id = ?")
            .bind(now)
            .bind(claim_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE player_claims SET status = 'declined', decided_at = ? \
             WHERE player_id = ? AND status = 'pending' AND id <> ?",
        )
        .bind(now)
        .bind(&claim.player_id)
        .bind(claim_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The coordinator declines: the entry stays a plain name.
    pub async fn decline_claim(
        &self,
        owner: &str,
        claim_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        pending_claim_in(&mut tx, Side::Owner, owner, claim_id).await?;
        decide(&mut tx, claim_id, "declined", now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The claimant's claims, newest first.
    pub async fn my_claims(&self, user_id: &str) -> Result<Vec<MyClaim>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let rows = sqlx::query(
            "SELECT id, player_id, status, created_at, decided_at FROM player_claims \
             WHERE claimant_user_id = ? ORDER BY created_at DESC, id",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            out.push(my_claim_of(&mut conn, row).await?);
        }
        Ok(out)
    }

    /// The claimant takes a pending claim back.
    pub async fn withdraw_claim(
        &self,
        user_id: &str,
        claim_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        pending_claim_in(&mut tx, Side::Claimant, user_id, claim_id).await?;
        decide(&mut tx, claim_id, "withdrawn", now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// How many pending claims wait on each of the owner's entries (disabled claimants left
    /// out, as in [`Store::incoming_claims`]), by player id.
    pub async fn pending_claim_counts(
        &self,
        owner: &str,
    ) -> Result<HashMap<String, u32>, StoreError> {
        let rows = sqlx::query(
            "SELECT c.player_id, COUNT(*) AS n FROM player_claims c \
             JOIN users u ON u.id = c.claimant_user_id \
             WHERE c.owner_user_id = ? AND c.status = 'pending' AND u.disabled_at IS NULL \
             GROUP BY c.player_id",
        )
        .bind(owner)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.try_get("player_id")?,
                    row.try_get::<i64, _>("n")?.try_into().unwrap_or(0),
                ))
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(Into::into)
    }
}

async fn decide(
    conn: &mut SqliteConnection,
    claim_id: &str,
    status: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE player_claims SET status = ?, decided_at = ? WHERE id = ?")
        .bind(status)
        .bind(now)
        .bind(claim_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Insert the claim row, or reset the one this person already has for the entry.
#[allow(clippy::too_many_arguments)]
async fn upsert(
    conn: &mut SqliteConnection,
    id: &str,
    player_id: &str,
    claimant: &str,
    owner: &str,
    status: &str,
    now: i64,
    decided_at: Option<i64>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO player_claims \
           (id, player_id, claimant_user_id, owner_user_id, status, created_at, decided_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (player_id, claimant_user_id) DO UPDATE SET \
           status = excluded.status, created_at = excluded.created_at, \
           decided_at = excluded.decided_at, owner_user_id = excluded.owner_user_id",
    )
    .bind(id)
    .bind(player_id)
    .bind(claimant)
    .bind(owner)
    .bind(status)
    .bind(now)
    .bind(decided_at)
    .execute(conn)
    .await?;
    Ok(())
}
