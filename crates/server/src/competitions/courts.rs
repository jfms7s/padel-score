//! Courts and event matches. An event match is an ordinary `matches` row (owned by the
//! coordinator, scored through its own tokens) plus a `competition_matches` link saying which
//! event and teams it belongs to and which court queue it sits in.
//!
//! A court holds one current match and a queue (`court_seq` order) behind it. A match that
//! leaves a court gets a fresh scorer token, so the court phone's old link stops working
//! (decision 3); the functions that do that report the match id, so the route layer can close
//! that match's open streams.
//!
//! The `*_in` functions run inside the caller's transaction and never begin or commit one, so
//! the kind modules can call them from their own hooks.

use padel_scoring::{Player, Start, Team};
use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use super::{Competition, Status, clean_name, competition_by_id_in, competition_in, team_in};
use crate::players::insert_match_players_in;
use crate::store::{
    NewMatch, Store, StoreError, Teams, Tx, create_match_in, rotate_scorer_token_in,
};
use crate::tokens::new_token;

/// The most courts one event holds.
pub const MAX_COURTS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Court {
    pub id: String,
    pub name: String,
    /// 1-based, in creation order.
    pub position: i64,
    pub scorer_token: String,
    pub spectator_token: String,
    pub current_match_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventMatchRow {
    pub match_id: String,
    pub event_id: String,
    pub team_a_id: String,
    pub team_b_id: String,
    pub label: String,
    /// 1-based creation order within the event.
    pub ordinal: i64,
    /// The court whose queue the match is (or was last) in. A finished match keeps it.
    pub court_id: Option<String>,
    pub court_seq: Option<i64>,
}

/// What advancing a court did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advanced {
    /// The court's new current match, if its queue had one.
    pub current: Option<String>,
    /// The old current match, whose scorer token was rotated.
    pub rotated: Option<String>,
}

fn court_from_row(row: &SqliteRow) -> Result<Court, sqlx::Error> {
    Ok(Court {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        position: row.try_get("position")?,
        scorer_token: row.try_get("scorer_token")?,
        spectator_token: row.try_get("spectator_token")?,
        current_match_id: row.try_get("current_match_id")?,
    })
}

fn event_match_from_row(row: &SqliteRow) -> Result<EventMatchRow, sqlx::Error> {
    Ok(EventMatchRow {
        match_id: row.try_get("match_id")?,
        event_id: row.try_get("event_id")?,
        team_a_id: row.try_get("team_a_id")?,
        team_b_id: row.try_get("team_b_id")?,
        label: row.try_get("label")?,
        ordinal: row.try_get("ordinal")?,
        court_id: row.try_get("court_id")?,
        court_seq: row.try_get("court_seq")?,
    })
}

macro_rules! select_courts {
    ($rest:literal) => {
        concat!(
            "SELECT id, name, position, scorer_token, spectator_token, current_match_id \
             FROM courts ",
            $rest
        )
    };
}

macro_rules! select_event_matches {
    ($rest:literal) => {
        concat!(
            "SELECT match_id, event_id, team_a_id, team_b_id, label, ordinal, court_id, \
             court_seq FROM competition_matches ",
            $rest
        )
    };
}

/// The event's courts in position order. The caller has already checked the event is theirs.
pub async fn courts_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Vec<Court>, StoreError> {
    let rows = sqlx::query(select_courts!("WHERE event_id = ? ORDER BY position, id"))
        .bind(event_id)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(court_from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

async fn court_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    court_id: &str,
) -> Result<Option<Court>, StoreError> {
    let row = sqlx::query(select_courts!("WHERE event_id = ? AND id = ?"))
        .bind(event_id)
        .bind(court_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(court_from_row).transpose()?)
}

/// The event's matches in creation order. The caller has already checked the event is theirs.
pub async fn event_matches_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Vec<EventMatchRow>, StoreError> {
    let rows = sqlx::query(select_event_matches!("WHERE event_id = ? ORDER BY ordinal"))
        .bind(event_id)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(event_match_from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

async fn event_match_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    match_id: &str,
) -> Result<Option<EventMatchRow>, StoreError> {
    let row = sqlx::query(select_event_matches!("WHERE event_id = ? AND match_id = ?"))
        .bind(event_id)
        .bind(match_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(event_match_from_row).transpose()?)
}

/// The event a match belongs to, whoever owns it, or `None` for a quick match. For callers
/// that came in by a match token; a coordinator route checks the owner itself.
pub async fn event_of_match_in(
    conn: &mut SqliteConnection,
    match_id: &str,
) -> Result<Option<(Competition, EventMatchRow)>, StoreError> {
    let row = sqlx::query(select_event_matches!("WHERE match_id = ?"))
        .bind(match_id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(link) = row.as_ref().map(event_match_from_row).transpose()? else {
        return Ok(None);
    };
    let ev = competition_by_id_in(conn, &link.event_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok(Some((ev, link)))
}

/// A match is finished when it has a `match_results` row.
async fn is_finished_in(conn: &mut SqliteConnection, match_id: &str) -> Result<bool, StoreError> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
            .bind(match_id)
            .fetch_optional(conn)
            .await?;
    Ok(found.is_some())
}

/// The court's next match: the lowest `court_seq` match queued there that is unfinished and
/// not current anywhere.
async fn next_queued_in(
    conn: &mut SqliteConnection,
    court_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT cm.match_id FROM competition_matches cm WHERE cm.court_id = ? \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = cm.match_id) \
         AND NOT EXISTS (SELECT 1 FROM courts c WHERE c.current_match_id = cm.match_id) \
         ORDER BY cm.court_seq, cm.ordinal LIMIT 1",
    )
    .bind(court_id)
    .fetch_optional(conn)
    .await?)
}

async fn set_current_in(
    tx: &mut Tx<'_>,
    court_id: &str,
    match_id: Option<&str>,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE courts SET current_match_id = ? WHERE id = ?")
        .bind(match_id)
        .bind(court_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Create an event match: a `matches` row owned by the coordinator in the event's format,
/// named from the teams' players as they are now, with all four `match_players` rows, then its
/// `competition_matches` link, then (with `court_id`) its court assignment.
///
/// The event must be `Running` (`Conflict("event_not_running")`); the teams must be two
/// different teams of this event (`InvalidField("teams")`). The default label is
/// "Match {ordinal}". Archived players are allowed: the team was valid when it was made.
pub async fn create_event_match_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    team_a: &str,
    team_b: &str,
    label: Option<String>,
    court_id: Option<&str>,
    now: i64,
) -> Result<EventMatchRow, StoreError> {
    if ev.status != Status::Running {
        return Err(StoreError::Conflict("event_not_running"));
    }
    if team_a == team_b {
        return Err(StoreError::InvalidField("teams"));
    }
    let a = team_in(tx, &ev.id, team_a)
        .await?
        .ok_or(StoreError::InvalidField("teams"))?;
    let b = team_in(tx, &ev.id, team_b)
        .await?
        .ok_or(StoreError::InvalidField("teams"))?;
    let label = label
        .as_deref()
        .map(|l| clean_name(l, "label"))
        .transpose()?;
    let ordinal: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM competition_matches WHERE event_id = ?",
    )
    .bind(&ev.id)
    .fetch_one(&mut **tx)
    .await?;
    let label = label.unwrap_or_else(|| format!("Match {ordinal}"));
    let [a1, a2] = &a.players;
    let [b1, b2] = &b.players;
    let m = create_match_in(
        tx,
        NewMatch {
            format: ev.format,
            start: Start {
                first_server: Player {
                    team: Team::A,
                    index: 0,
                },
                other_first_server: 0,
            },
            teams: Teams {
                a: [a1.name.clone(), a2.name.clone()],
                b: [b1.name.clone(), b2.name.clone()],
            },
            owner_user_id: Some(ev.owner_user_id.clone()),
            players: Default::default(),
        },
        now,
    )
    .await?;
    insert_match_players_in(
        tx,
        &m.id,
        &[
            (Team::A, 0, a1.id.as_str()),
            (Team::A, 1, a2.id.as_str()),
            (Team::B, 0, b1.id.as_str()),
            (Team::B, 1, b2.id.as_str()),
        ],
    )
    .await?;
    sqlx::query(
        "INSERT INTO competition_matches (match_id, event_id, team_a_id, team_b_id, label, \
         ordinal) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(&ev.id)
    .bind(&a.id)
    .bind(&b.id)
    .bind(&label)
    .bind(ordinal)
    .execute(&mut **tx)
    .await?;
    if court_id.is_some() {
        // A new match was on no court, so nothing is rotated.
        assign_in(tx, ev, &m.id, court_id).await?;
    }
    event_match_in(tx, &ev.id, &m.id)
        .await?
        .ok_or(StoreError::NotFound)
}

/// Put the match in `court_id`'s queue, or with `None` take it off any court. Returns the id of
/// the match if its scorer token was rotated (it was current on a court and left it).
///
/// - An unknown match of this event is `NotFound`; an unknown court is `InvalidField("court_id")`.
/// - A finished match is `Conflict("match_finished")`; a finished event `Conflict("event_finished")`.
/// - Assigning a match to the court it is already on changes nothing.
/// - A match leaving the court it is current on gets a fresh scorer token, and that court's
///   next queued match becomes current (or none).
/// - On the new court it gets `court_seq = max + 1`, and is current if the court has none.
pub async fn assign_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
    court_id: Option<&str>,
) -> Result<Option<String>, StoreError> {
    if ev.status == Status::Finished {
        return Err(StoreError::Conflict("event_finished"));
    }
    let row = event_match_in(tx, &ev.id, match_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    if is_finished_in(tx, match_id).await? {
        return Err(StoreError::Conflict("match_finished"));
    }
    let target = match court_id {
        Some(id) => Some(
            court_in(tx, &ev.id, id)
                .await?
                .ok_or(StoreError::InvalidField("court_id"))?,
        ),
        None => None,
    };
    if row.court_id.as_deref() == court_id {
        return Ok(None);
    }
    let current_on: Option<String> =
        sqlx::query_scalar("SELECT id FROM courts WHERE current_match_id = ?")
            .bind(match_id)
            .fetch_optional(&mut **tx)
            .await?;
    let mut rotated = None;
    if let Some(old) = &current_on {
        set_current_in(tx, old, None).await?;
        rotate_scorer_token_in(tx, match_id).await?;
        rotated = Some(match_id.to_owned());
    }
    let seq: Option<i64> = match &target {
        Some(court) => Some(
            sqlx::query_scalar(
                "SELECT COALESCE(MAX(court_seq), 0) + 1 FROM competition_matches \
                 WHERE court_id = ?",
            )
            .bind(&court.id)
            .fetch_one(&mut **tx)
            .await?,
        ),
        None => None,
    };
    sqlx::query("UPDATE competition_matches SET court_id = ?, court_seq = ? WHERE match_id = ?")
        .bind(court_id)
        .bind(seq)
        .bind(match_id)
        .execute(&mut **tx)
        .await?;
    if let Some(old) = &current_on {
        let next = next_queued_in(tx, old).await?;
        set_current_in(tx, old, next.as_deref()).await?;
    }
    if let Some(court) = target
        && court.current_match_id.is_none()
    {
        set_current_in(tx, &court.id, Some(match_id)).await?;
    }
    Ok(rotated)
}

/// Move the court on: the current match must be finished or absent
/// (`Conflict("match_not_finished")`). The old current match gets a fresh scorer token, and the
/// lowest `court_seq` unfinished queued match becomes current. An unknown court of this event
/// is `NotFound`; a finished event `Conflict("event_finished")`.
pub async fn advance_court_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    court_id: &str,
) -> Result<Advanced, StoreError> {
    if ev.status == Status::Finished {
        return Err(StoreError::Conflict("event_finished"));
    }
    let court = court_in(tx, &ev.id, court_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let mut rotated = None;
    if let Some(current) = court.current_match_id {
        if !is_finished_in(tx, &current).await? {
            return Err(StoreError::Conflict("match_not_finished"));
        }
        set_current_in(tx, court_id, None).await?;
        rotate_scorer_token_in(tx, &current).await?;
        rotated = Some(current);
    }
    let next = next_queued_in(tx, court_id).await?;
    set_current_in(tx, court_id, next.as_deref()).await?;
    Ok(Advanced {
        current: next,
        rotated,
    })
}

async fn owned_event_in(
    conn: &mut SqliteConnection,
    owner: &str,
    event_id: &str,
) -> Result<Competition, StoreError> {
    competition_in(conn, owner, event_id)
        .await?
        .ok_or(StoreError::NotFound)
}

impl Store {
    /// Add a court; without a name it is "Court {n}", `n` being its position. Not once the event
    /// is finished (`Conflict("event_finished")`), nor past [`MAX_COURTS`].
    pub async fn add_court(
        &self,
        owner: &str,
        event_id: &str,
        name: Option<String>,
    ) -> Result<Court, StoreError> {
        let name = name.as_deref().map(|n| clean_name(n, "name")).transpose()?;
        let mut tx = self.begin().await?;
        let ev = owned_event_in(&mut tx, owner, event_id).await?;
        if ev.status == Status::Finished {
            return Err(StoreError::Conflict("event_finished"));
        }
        let (count, position): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(MAX(position), 0) + 1 FROM courts WHERE event_id = ?",
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;
        if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_COURTS {
            return Err(StoreError::Conflict("too_many_courts"));
        }
        let court = Court {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.unwrap_or_else(|| format!("Court {position}")),
            position,
            scorer_token: new_token(),
            spectator_token: new_token(),
            current_match_id: None,
        };
        sqlx::query(
            "INSERT INTO courts (id, event_id, name, position, scorer_token, spectator_token) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&court.id)
        .bind(event_id)
        .bind(&court.name)
        .bind(court.position)
        .bind(&court.scorer_token)
        .bind(&court.spectator_token)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(court)
    }

    pub async fn rename_court(
        &self,
        owner: &str,
        event_id: &str,
        court_id: &str,
        name: String,
    ) -> Result<Court, StoreError> {
        let name = clean_name(&name, "name")?;
        let mut tx = self.begin().await?;
        owned_event_in(&mut tx, owner, event_id).await?;
        let mut court = court_in(&mut tx, event_id, court_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        sqlx::query("UPDATE courts SET name = ? WHERE id = ?")
            .bind(&name)
            .bind(court_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        court.name = name;
        Ok(court)
    }

    /// Delete a court no event match is on (queued, current or played there), else
    /// `Conflict("court_in_use")`.
    pub async fn delete_court(
        &self,
        owner: &str,
        event_id: &str,
        court_id: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        owned_event_in(&mut tx, owner, event_id).await?;
        let court = court_in(&mut tx, event_id, court_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        let used: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM competition_matches WHERE court_id = ? LIMIT 1")
                .bind(court_id)
                .fetch_optional(&mut *tx)
                .await?;
        if court.current_match_id.is_some() || used.is_some() {
            return Err(StoreError::Conflict("court_in_use"));
        }
        sqlx::query("DELETE FROM courts WHERE id = ?")
            .bind(court_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reissue the court's scoring code: a new court scorer token, and a new scorer token for
    /// its current match (the returned `current_match_id`, whose streams the caller closes).
    /// The spectator token stays.
    pub async fn rotate_court(
        &self,
        owner: &str,
        event_id: &str,
        court_id: &str,
    ) -> Result<Court, StoreError> {
        let mut tx = self.begin().await?;
        owned_event_in(&mut tx, owner, event_id).await?;
        let mut court = court_in(&mut tx, event_id, court_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        court.scorer_token = new_token();
        sqlx::query("UPDATE courts SET scorer_token = ? WHERE id = ?")
            .bind(&court.scorer_token)
            .bind(court_id)
            .execute(&mut *tx)
            .await?;
        if let Some(current) = &court.current_match_id {
            rotate_scorer_token_in(&mut tx, current).await?;
        }
        tx.commit().await?;
        Ok(court)
    }

    /// See [`create_event_match_in`]. An event that is not the owner's is `NotFound`.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_event_match(
        &self,
        owner: &str,
        event_id: &str,
        team_a: &str,
        team_b: &str,
        label: Option<String>,
        court_id: Option<&str>,
        now: i64,
    ) -> Result<EventMatchRow, StoreError> {
        let mut tx = self.begin().await?;
        let ev = owned_event_in(&mut tx, owner, event_id).await?;
        let row = create_event_match_in(&mut tx, &ev, team_a, team_b, label, court_id, now).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// See [`assign_in`]. Returns the match id if its scorer token was rotated.
    pub async fn assign_match(
        &self,
        owner: &str,
        event_id: &str,
        match_id: &str,
        court_id: Option<&str>,
    ) -> Result<Option<String>, StoreError> {
        let mut tx = self.begin().await?;
        let ev = owned_event_in(&mut tx, owner, event_id).await?;
        let rotated = assign_in(&mut tx, &ev, match_id, court_id).await?;
        tx.commit().await?;
        Ok(rotated)
    }

    /// The coordinator moves a court on. See [`advance_court_in`].
    pub async fn advance_court(
        &self,
        owner: &str,
        event_id: &str,
        court_id: &str,
    ) -> Result<Advanced, StoreError> {
        let mut tx = self.begin().await?;
        let ev = owned_event_in(&mut tx, owner, event_id).await?;
        let done = advance_court_in(&mut tx, &ev, court_id).await?;
        tx.commit().await?;
        Ok(done)
    }

    /// The court phone moves its court on. An unknown court token is `NotFound`. See
    /// [`advance_court_in`].
    pub async fn advance_court_by_token(&self, court_token: &str) -> Result<Advanced, StoreError> {
        let mut tx = self.begin().await?;
        let found: Option<(String, String)> =
            sqlx::query_as("SELECT id, event_id FROM courts WHERE scorer_token = ?")
                .bind(court_token)
                .fetch_optional(&mut *tx)
                .await?;
        let (court_id, event_id) = found.ok_or(StoreError::NotFound)?;
        let ev = competition_by_id_in(&mut tx, &event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        let done = advance_court_in(&mut tx, &ev, &court_id).await?;
        tx.commit().await?;
        Ok(done)
    }
}
