//! Events: a coordinator's tournament or non-stop session, and the teams in it. "Event" is the
//! UI word; here it is a competition, because the `events` table is the per-match point log.
//!
//! The kind-specific parts (tournament, non-stop) sit behind [`kinds`]; this module holds what
//! both share. Like `players.rs`, it takes `now` as a parameter, writes inside
//! [`Store::begin`], and exposes `*_in` variants for callers already in a transaction.

pub mod courts;
pub mod kinds;
pub mod non_stop;
pub mod places;
pub mod tournament;
pub mod view;

use padel_scoring::Format;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::players::player_in;
use crate::store::{MAX_NAME_CHARS, Store, StoreError, StoredMatch, Tx, decode, encode};
use crate::tokens::new_token;

/// The most teams one event holds.
pub const MAX_TEAMS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Tournament,
    NonStop,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Tournament => "tournament",
            Kind::NonStop => "non_stop",
        }
    }

    fn parse(s: &str) -> Result<Kind, sqlx::Error> {
        match s {
            "tournament" => Ok(Kind::Tournament),
            "non_stop" => Ok(Kind::NonStop),
            other => Err(sqlx::Error::Decode(
                format!("unknown event kind {other:?}").into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Setup,
    Running,
    Finished,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Setup => "setup",
            Status::Running => "running",
            Status::Finished => "finished",
        }
    }

    fn parse(s: &str) -> Result<Status, sqlx::Error> {
        match s {
            "setup" => Ok(Status::Setup),
            "running" => Ok(Status::Running),
            "finished" => Ok(Status::Finished),
            other => Err(sqlx::Error::Decode(
                format!("unknown event status {other:?}").into(),
            )),
        }
    }
}

/// Who a kind's view is built for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
    Coordinator,
    Court { court_id: String },
    Spectator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Competition {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub kind: Kind,
    pub format: Format,
    pub settings: serde_json::Value,
    pub status: Status,
    pub spectator_token: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewCompetition {
    pub name: String,
    pub kind: Kind,
    pub format: Format,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CompetitionChange {
    pub name: Option<String>,
    pub format: Option<Format>,
    pub settings: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamPlayer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventTeam {
    pub id: String,
    /// The shown name: the custom name, or else "{p1} / {p2}" from the players' current names.
    pub name: String,
    pub custom_name: Option<String>,
    pub players: [TeamPlayer; 2],
    pub position: i64,
}

/// `name: Some(None)` clears the custom name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TeamChange {
    pub name: Option<Option<String>>,
    pub players: Option<[String; 2]>,
}

/// Settings of a kind that has none yet: absent or `{}` gives `{}`, anything else is refused.
pub(crate) fn empty_settings(
    raw: Option<&serde_json::Value>,
) -> Result<serde_json::Value, StoreError> {
    match raw {
        None => Ok(serde_json::json!({})),
        Some(v) if v.as_object().is_some_and(serde_json::Map::is_empty) => Ok(v.clone()),
        Some(_) => Err(StoreError::InvalidField("settings")),
    }
}

/// Trim a name; blank or over 40 chars (not bytes) is `InvalidField(field)`.
fn clean_name(raw: &str, field: &'static str) -> Result<String, StoreError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::InvalidField(field));
    }
    Ok(name.to_owned())
}

/// A tournament is played in sets, a non-stop session on the clock.
fn check_format(kind: Kind, format: &Format) -> Result<(), StoreError> {
    let fits = matches!(
        (kind, format),
        (Kind::Tournament, Format::Sets(_)) | (Kind::NonStop, Format::Timed { .. })
    );
    if !fits {
        return Err(StoreError::InvalidField("format"));
    }
    format.validate()?;
    Ok(())
}

macro_rules! select_competitions {
    ($rest:literal) => {
        concat!(
            "SELECT id, owner_user_id, name, kind, format, settings, status, spectator_token, \
             created_at, started_at, finished_at FROM competitions ",
            $rest
        )
    };
}

fn competition_from_row(row: &SqliteRow) -> Result<Competition, sqlx::Error> {
    Ok(Competition {
        id: row.try_get("id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        name: row.try_get("name")?,
        kind: Kind::parse(row.try_get::<&str, _>("kind")?)?,
        format: decode(row.try_get::<&str, _>("format")?)?,
        settings: decode(row.try_get::<&str, _>("settings")?)?,
        status: Status::parse(row.try_get::<&str, _>("status")?)?,
        spectator_token: row.try_get("spectator_token")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

/// The owner's event `id`; another owner's, like an unknown one, is `None`.
pub async fn competition_in(
    conn: &mut SqliteConnection,
    owner: &str,
    id: &str,
) -> Result<Option<Competition>, StoreError> {
    let row = sqlx::query(select_competitions!("WHERE owner_user_id = ? AND id = ?"))
        .bind(owner)
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(competition_from_row).transpose()?)
}

/// The event `id` whoever owns it: for a caller that came in by a token, not as the owner.
pub(crate) async fn competition_by_id_in(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Competition>, StoreError> {
    let row = sqlx::query(select_competitions!("WHERE id = ?"))
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(competition_from_row).transpose()?)
}

/// The event hook of a command that changed a match's log, run in its transaction after
/// `results::sync_in`: for an event match it calls [`kinds::on_result`], which reads
/// `match_results` to see whether the match is now finished (or no longer is, after an undo).
/// For any other match it does nothing beyond one indexed lookup.
pub async fn after_result_in(tx: &mut Tx<'_>, m: &StoredMatch, now: i64) -> Result<(), StoreError> {
    if let Some((ev, link)) = courts::event_of_match_in(tx, &m.id).await? {
        kinds::on_result(ev.kind, tx, &ev, &link.match_id, now).await?;
    }
    Ok(())
}

fn team_from_row(row: &SqliteRow) -> Result<EventTeam, sqlx::Error> {
    let players = [
        TeamPlayer {
            id: row.try_get("p1_id")?,
            name: row.try_get("p1_name")?,
        },
        TeamPlayer {
            id: row.try_get("p2_id")?,
            name: row.try_get("p2_name")?,
        },
    ];
    let custom_name: Option<String> = row.try_get("name")?;
    Ok(EventTeam {
        id: row.try_get("id")?,
        name: custom_name
            .clone()
            .unwrap_or_else(|| format!("{} / {}", players[0].name, players[1].name)),
        custom_name,
        players,
        position: row.try_get("position")?,
    })
}

macro_rules! select_teams {
    ($rest:literal) => {
        concat!(
            "SELECT t.id, t.name, t.position, p1.id AS p1_id, p1.name AS p1_name, \
             p2.id AS p2_id, p2.name AS p2_name FROM competition_teams t \
             JOIN players p1 ON p1.id = t.player1_id \
             JOIN players p2 ON p2.id = t.player2_id ",
            $rest
        )
    };
}

/// The event's teams in position order. The caller has already checked the event is theirs.
pub async fn teams_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Vec<EventTeam>, StoreError> {
    let rows = sqlx::query(select_teams!(
        "WHERE t.event_id = ? ORDER BY t.position, t.id"
    ))
    .bind(event_id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .iter()
        .map(team_from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

async fn team_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    team_id: &str,
) -> Result<Option<EventTeam>, StoreError> {
    let row = sqlx::query(select_teams!("WHERE t.event_id = ? AND t.id = ?"))
        .bind(event_id)
        .bind(team_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(team_from_row).transpose()?)
}

/// Both players are the owner's, active and different, and in no other team of the event
/// (`except_team` is the team being re-paired, if any).
async fn check_pair(
    conn: &mut SqliteConnection,
    owner: &str,
    event_id: &str,
    pair: &[String; 2],
    except_team: Option<&str>,
) -> Result<(), StoreError> {
    if pair[0] == pair[1] {
        return Err(StoreError::InvalidField("players"));
    }
    for id in pair {
        match player_in(conn, owner, id).await? {
            Some(p) if !p.archived => {}
            _ => return Err(StoreError::InvalidField("players")),
        }
    }
    let taken: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM competition_teams WHERE event_id = ? AND id IS NOT ? \
         AND (player1_id IN (?, ?) OR player2_id IN (?, ?)) LIMIT 1",
    )
    .bind(event_id)
    .bind(except_team)
    .bind(&pair[0])
    .bind(&pair[1])
    .bind(&pair[0])
    .bind(&pair[1])
    .fetch_optional(conn)
    .await?;
    if taken.is_some() {
        return Err(StoreError::Conflict("player_in_team"));
    }
    Ok(())
}

impl Store {
    pub async fn create_competition(
        &self,
        owner: &str,
        new: NewCompetition,
        now: i64,
    ) -> Result<Competition, StoreError> {
        let name = clean_name(&new.name, "name")?;
        check_format(new.kind, &new.format)?;
        let settings = kinds::normalize_settings(new.kind, new.settings.as_ref())?;
        let ev = Competition {
            id: uuid::Uuid::new_v4().to_string(),
            owner_user_id: owner.to_owned(),
            name,
            kind: new.kind,
            format: new.format,
            settings,
            status: Status::Setup,
            spectator_token: new_token(),
            created_at: now,
            started_at: None,
            finished_at: None,
        };
        let mut tx = self.begin().await?;
        sqlx::query(
            "INSERT INTO competitions (id, owner_user_id, name, kind, format, settings, status, \
             spectator_token, created_at) VALUES (?, ?, ?, ?, ?, ?, 'setup', ?, ?)",
        )
        .bind(&ev.id)
        .bind(owner)
        .bind(&ev.name)
        .bind(ev.kind.as_str())
        .bind(encode(&ev.format)?)
        .bind(encode(&ev.settings)?)
        .bind(&ev.spectator_token)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// The owner's events, newest first, each with its team and court counts.
    pub async fn competitions(
        &self,
        owner: &str,
    ) -> Result<Vec<(Competition, usize, usize)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, owner_user_id, name, kind, format, settings, status, spectator_token, \
             created_at, started_at, finished_at, \
             (SELECT COUNT(*) FROM competition_teams WHERE event_id = competitions.id) AS teams, \
             (SELECT COUNT(*) FROM courts WHERE event_id = competitions.id) AS courts \
             FROM competitions WHERE owner_user_id = ? ORDER BY created_at DESC, rowid DESC",
        )
        .bind(owner)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                let teams: i64 = row.try_get("teams")?;
                let courts: i64 = row.try_get("courts")?;
                Ok((
                    competition_from_row(row)?,
                    usize::try_from(teams).unwrap_or(0),
                    usize::try_from(courts).unwrap_or(0),
                ))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(StoreError::from)
    }

    pub async fn competition(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<Competition>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        competition_in(&mut conn, owner, id).await
    }

    /// Rename, and in `setup` only change the format or settings (else `Conflict("event_started")`).
    pub async fn update_competition(
        &self,
        owner: &str,
        id: &str,
        change: CompetitionChange,
    ) -> Result<Competition, StoreError> {
        let name = change
            .name
            .as_deref()
            .map(|n| clean_name(n, "name"))
            .transpose()?;
        let mut tx = self.begin().await?;
        let mut ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if (change.format.is_some() || change.settings.is_some()) && ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        if let Some(name) = name {
            ev.name = name;
        }
        if let Some(format) = change.format {
            ev.format = format;
        }
        check_format(ev.kind, &ev.format)?;
        if change.settings.is_some() {
            ev.settings = kinds::normalize_settings(ev.kind, change.settings.as_ref())?;
        }
        sqlx::query("UPDATE competitions SET name = ?, format = ?, settings = ? WHERE id = ?")
            .bind(&ev.name)
            .bind(encode(&ev.format)?)
            .bind(encode(&ev.settings)?)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// Delete an event still in `setup` (it has no matches yet), with its teams and courts.
    pub async fn delete_competition(&self, owner: &str, id: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        sqlx::query("DELETE FROM competitions WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The event's teams, or `None` when it is not the owner's.
    pub async fn teams(
        &self,
        owner: &str,
        event_id: &str,
    ) -> Result<Option<Vec<EventTeam>>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        if competition_in(&mut conn, owner, event_id).await?.is_none() {
            return Ok(None);
        }
        Ok(Some(teams_in(&mut conn, event_id).await?))
    }

    pub async fn add_team(
        &self,
        owner: &str,
        event_id: &str,
        players: [String; 2],
        name: Option<String>,
        _now: i64,
    ) -> Result<EventTeam, StoreError> {
        let name = name.as_deref().map(|n| clean_name(n, "name")).transpose()?;
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        check_pair(&mut tx, owner, event_id, &players, None).await?;
        let (count, next): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(MAX(position) + 1, 0) FROM competition_teams \
             WHERE event_id = ?",
        )
        .bind(event_id)
        .fetch_one(&mut *tx)
        .await?;
        if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_TEAMS {
            return Err(StoreError::Conflict("too_many_teams"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO competition_teams (id, event_id, name, player1_id, player2_id, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(event_id)
        .bind(&name)
        .bind(&players[0])
        .bind(&players[1])
        .bind(next)
        .execute(&mut *tx)
        .await?;
        let team = team_in(&mut tx, event_id, &id)
            .await?
            .ok_or(StoreError::NotFound)?;
        tx.commit().await?;
        Ok(team)
    }

    /// Rename (until the event is finished) and/or re-pair (in `setup` only).
    pub async fn update_team(
        &self,
        owner: &str,
        event_id: &str,
        team_id: &str,
        change: TeamChange,
    ) -> Result<EventTeam, StoreError> {
        let name = change
            .name
            .as_ref()
            .map(|n| n.as_deref().map(|n| clean_name(n, "name")).transpose())
            .transpose()?;
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        let mut team = team_in(&mut tx, event_id, team_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if change.players.is_some() && ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        if name.is_some() && ev.status == Status::Finished {
            return Err(StoreError::Conflict("event_finished"));
        }
        if let Some(pair) = &change.players {
            check_pair(&mut tx, owner, event_id, pair, Some(team_id)).await?;
            sqlx::query("UPDATE competition_teams SET player1_id = ?, player2_id = ? WHERE id = ?")
                .bind(&pair[0])
                .bind(&pair[1])
                .bind(team_id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(name) = &name {
            sqlx::query("UPDATE competition_teams SET name = ? WHERE id = ?")
                .bind(name)
                .bind(team_id)
                .execute(&mut *tx)
                .await?;
        }
        if change.players.is_some() || name.is_some() {
            team = team_in(&mut tx, event_id, team_id)
                .await?
                .ok_or(StoreError::NotFound)?;
        }
        tx.commit().await?;
        Ok(team)
    }

    pub async fn delete_team(
        &self,
        owner: &str,
        event_id: &str,
        team_id: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        let done = sqlx::query("DELETE FROM competition_teams WHERE event_id = ? AND id = ?")
            .bind(event_id)
            .bind(team_id)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }
}
