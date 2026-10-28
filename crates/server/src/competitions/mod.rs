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
pub mod schedule;
pub mod signups;
pub mod tournament;
pub mod view;

use padel_scoring::Format;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::players::player_in;
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{MAX_NAME_CHARS, Store, StoreError, StoredMatch, Tx, decode, encode};
use crate::tokens::new_token;
use schedule::{Schedule, ScheduleInput, Signups, normalize_schedule};

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
    /// Called off before it started. Stored as `setup` with `cancelled_at` set.
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Setup => "setup",
            Status::Running => "running",
            Status::Finished => "finished",
            Status::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> Result<Status, sqlx::Error> {
        match s {
            "setup" => Ok(Status::Setup),
            "running" => Ok(Status::Running),
            "finished" => Ok(Status::Finished),
            "cancelled" => Ok(Status::Cancelled),
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
    pub schedule: Schedule,
    /// The `/j/{token}` link, made the first time sign-ups are turned on and kept after.
    pub join_token: Option<String>,
    pub cancelled_at: Option<i64>,
    /// When sign-ups closed: the deadline ran, or the organiser took the teams.
    pub signups_closed_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewCompetition {
    pub name: String,
    pub kind: Kind,
    pub format: Format,
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
    #[serde(default)]
    pub schedule: Option<ScheduleInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct CompetitionChange {
    pub name: Option<String>,
    pub format: Option<Format>,
    pub settings: Option<serde_json::Value>,
    #[serde(default)]
    pub schedule: Option<ScheduleInput>,
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

/// The `competitions` columns [`competition_from_row`] reads.
macro_rules! competition_columns {
    () => {
        "id, owner_user_id, name, kind, format, settings, status, spectator_token, \
         created_at, started_at, finished_at, starts_at, place, signups, min_teams, max_teams, \
         deadline_at, join_token, cancelled_at, signups_closed_at"
    };
}

macro_rules! select_competitions {
    ($rest:literal) => {
        concat!(
            "SELECT ",
            competition_columns!(),
            " FROM competitions ",
            $rest
        )
    };
}

/// An event that was cancelled is refused every write (`Conflict("event_cancelled")`). Reads
/// and deleting it stay allowed.
pub(crate) fn not_cancelled(ev: &Competition) -> Result<(), StoreError> {
    if ev.status == Status::Cancelled {
        return Err(StoreError::Conflict("event_cancelled"));
    }
    Ok(())
}

fn competition_from_row(row: &SqliteRow) -> Result<Competition, sqlx::Error> {
    let cancelled_at: Option<i64> = row.try_get("cancelled_at")?;
    let status = if cancelled_at.is_some() {
        Status::Cancelled
    } else {
        Status::parse(row.try_get::<&str, _>("status")?)?
    };
    Ok(Competition {
        id: row.try_get("id")?,
        owner_user_id: row.try_get("owner_user_id")?,
        name: row.try_get("name")?,
        kind: Kind::parse(row.try_get::<&str, _>("kind")?)?,
        format: decode(row.try_get::<&str, _>("format")?)?,
        settings: decode(row.try_get::<&str, _>("settings")?)?,
        status,
        spectator_token: row.try_get("spectator_token")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        schedule: Schedule {
            starts_at: row.try_get("starts_at")?,
            place: row.try_get("place")?,
            signups: Signups::parse(row.try_get::<&str, _>("signups")?)?,
            min_teams: row.try_get("min_teams")?,
            max_teams: row.try_get("max_teams")?,
            deadline_at: row.try_get("deadline_at")?,
        },
        join_token: row.try_get("join_token")?,
        cancelled_at,
        signups_closed_at: row.try_get("signups_closed_at")?,
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

/// The event whose join link is `token`, whoever owns it.
pub(crate) async fn competition_by_join_token_in(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<Option<Competition>, StoreError> {
    let row = sqlx::query(select_competitions!("WHERE join_token = ?"))
        .bind(token)
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
/// (`except_team` is the team being re-paired, if any). Entries linked to one account are one
/// person: the pair may not share an account, nor either share one with a player of another team.
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
    .fetch_optional(&mut *conn)
    .await?;
    if taken.is_some() {
        return Err(StoreError::Conflict("player_in_team"));
    }
    let same_person: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM players a JOIN players b \
           ON b.linked_user_id = a.linked_user_id AND b.id <> a.id \
           WHERE a.id = ?1 AND b.id = ?2) \
         OR EXISTS (SELECT 1 FROM players mine \
           JOIN players other ON other.linked_user_id = mine.linked_user_id \
           JOIN competition_teams ct ON other.id IN (ct.player1_id, ct.player2_id) \
           WHERE mine.id IN (?1, ?2) AND ct.event_id = ?3 AND ct.id IS NOT ?4)",
    )
    .bind(&pair[0])
    .bind(&pair[1])
    .bind(event_id)
    .bind(except_team)
    .fetch_one(conn)
    .await?;
    if same_person {
        return Err(StoreError::Conflict("player_in_team"));
    }
    Ok(())
}

/// Whether anyone has signed up for the event.
async fn has_signups_in(conn: &mut SqliteConnection, event_id: &str) -> Result<bool, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM event_signups WHERE event_id = ?)")
            .bind(event_id)
            .fetch_one(conn)
            .await?,
    )
}

/// Tell every user signed up for `ev` (but not its organiser) that it was cancelled or its
/// start or place changed. Queued in the caller's transaction; the HTTP layer wakes the push
/// sender. Nobody has signed up before sign-ups were first turned on, so no token means no one.
async fn notify_signed_up_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    kind: NoticeKind,
    now: i64,
) -> Result<(), StoreError> {
    let Some(token) = &ev.join_token else {
        return Ok(());
    };
    let users: Vec<String> = sqlx::query_scalar(
        "SELECT user1_id FROM event_signups WHERE event_id = ?1 \
         UNION SELECT user2_id FROM event_signups WHERE event_id = ?1 AND user2_id IS NOT NULL",
    )
    .bind(&ev.id)
    .fetch_all(&mut **tx)
    .await?;
    let notices: Vec<Notice> = users
        .into_iter()
        .filter(|user| *user != ev.owner_user_id)
        .map(|user_id| Notice {
            user_id,
            payload: Payload {
                kind,
                url: format!("/j/{token}"),
                title: Some(ev.name.clone()),
                start: ev.schedule.starts_at,
                ..Payload::default()
            },
            dedupe: None,
        })
        .collect();
    queue_in(tx, &notices, now).await?;
    Ok(())
}

/// Add a team of two of the owner's players to `ev` (in setup, not cancelled: the caller
/// checks) inside the caller's transaction. The pair rules are [`check_pair`]'s
/// (`Conflict("player_in_team")` for a player already in a team, or one account twice), and
/// `Conflict("too_many_teams")` past [`MAX_TEAMS`]. `name` is already cleaned.
pub(crate) async fn add_team_in(
    tx: &mut Tx<'_>,
    owner: &str,
    ev: &Competition,
    players: [String; 2],
    name: Option<String>,
) -> Result<EventTeam, StoreError> {
    let event_id = ev.id.as_str();
    check_pair(tx, owner, event_id, &players, None).await?;
    let (count, next): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(MAX(position) + 1, 0) FROM competition_teams \
         WHERE event_id = ?",
    )
    .bind(event_id)
    .fetch_one(&mut **tx)
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
    .execute(&mut **tx)
    .await?;
    team_in(tx, event_id, &id)
        .await?
        .ok_or(StoreError::NotFound)
}

/// Call `ev` off as of `now`: mark it cancelled, close its sign-ups and tell the signed-up users
/// (not the organiser).
pub(crate) async fn cancel_in(
    tx: &mut Tx<'_>,
    ev: &mut Competition,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE competitions SET cancelled_at = ?, \
         signups_closed_at = COALESCE(signups_closed_at, ?) WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(&ev.id)
    .execute(&mut **tx)
    .await?;
    ev.status = Status::Cancelled;
    ev.cancelled_at = Some(now);
    ev.signups_closed_at = ev.signups_closed_at.or(Some(now));
    notify_signed_up_in(tx, ev, NoticeKind::EventCancelled, now).await
}

/// The sign-up layout before a write that can move teams across the maximum line, or `None`
/// for an event that never took sign-ups.
async fn layout_before(
    tx: &mut Tx<'_>,
    ev: &Competition,
) -> Result<Option<signups::Layout>, StoreError> {
    if ev.join_token.is_none() {
        return Ok(None);
    }
    Ok(Some(
        signups::layout_in(tx, &ev.id, ev.schedule.max_teams).await?,
    ))
}

/// Tell the sign-ups that crossed the maximum line since `before` (taken by [`layout_before`]
/// on the event as it was) that they moved, apart from `actor`.
async fn tell_moves(
    tx: &mut Tx<'_>,
    ev: &Competition,
    before: Option<signups::Layout>,
    actor: &str,
    now: i64,
) -> Result<(), StoreError> {
    let Some(before) = before else {
        return Ok(());
    };
    let after = signups::layout_in(tx, &ev.id, ev.schedule.max_teams).await?;
    signups::queue_moves_in(tx, ev, &before, &after, actor, now).await
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
        let schedule = normalize_schedule(
            new.kind,
            &settings,
            &Schedule::default(),
            &new.schedule.unwrap_or_default(),
            now,
        )?;
        let join_token = (schedule.signups != Signups::Off).then(new_token);
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
            schedule,
            join_token,
            cancelled_at: None,
            signups_closed_at: None,
        };
        let mut tx = self.begin().await?;
        sqlx::query(
            "INSERT INTO competitions (id, owner_user_id, name, kind, format, settings, status, \
             spectator_token, created_at, starts_at, place, signups, min_teams, max_teams, \
             deadline_at, join_token) \
             VALUES (?, ?, ?, ?, ?, ?, 'setup', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ev.id)
        .bind(owner)
        .bind(&ev.name)
        .bind(ev.kind.as_str())
        .bind(encode(&ev.format)?)
        .bind(encode(&ev.settings)?)
        .bind(&ev.spectator_token)
        .bind(now)
        .bind(ev.schedule.starts_at)
        .bind(&ev.schedule.place)
        .bind(ev.schedule.signups.as_str())
        .bind(ev.schedule.min_teams)
        .bind(ev.schedule.max_teams)
        .bind(ev.schedule.deadline_at)
        .bind(&ev.join_token)
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
        let rows = sqlx::query(concat!(
            "SELECT ",
            competition_columns!(),
            ", (SELECT COUNT(*) FROM competition_teams WHERE event_id = competitions.id) AS teams, \
                 (SELECT COUNT(*) FROM courts WHERE event_id = competitions.id) AS courts \
                 FROM competitions WHERE owner_user_id = ? ORDER BY created_at DESC, rowid DESC"
        ))
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

    /// Rename, and in `setup` only change the format, settings or schedule (else
    /// `Conflict("event_started")`). A cancelled event is `Conflict("event_cancelled")`.
    ///
    /// Turning sign-ups on the first time makes the join token. Turning them off while
    /// sign-ups exist is `Conflict("signups_exist")`. A new start or place tells the signed-up
    /// users (not the organiser).
    pub async fn update_competition(
        &self,
        owner: &str,
        id: &str,
        change: CompetitionChange,
        now: i64,
    ) -> Result<Competition, StoreError> {
        let mut tx = self.begin().await?;
        let mut ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        not_cancelled(&ev)?;
        let name = change
            .name
            .as_deref()
            .map(|n| clean_name(n, "name"))
            .transpose()?;
        let edits_setup =
            change.format.is_some() || change.settings.is_some() || change.schedule.is_some();
        if edits_setup && ev.status != Status::Setup {
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
        let before = ev.schedule.clone();
        let lines_before = layout_before(&mut tx, &ev).await?;
        if change.schedule.is_some() || change.settings.is_some() {
            // Settings are checked too: fewer legs may no longer fit the maximum.
            let input = change.schedule.unwrap_or_default();
            ev.schedule = normalize_schedule(ev.kind, &ev.settings, &before, &input, now)?;
        }
        if ev.schedule.signups == Signups::Off
            && before.signups != Signups::Off
            && has_signups_in(&mut tx, id).await?
        {
            return Err(StoreError::Conflict("signups_exist"));
        }
        if ev.schedule.signups != Signups::Off && ev.join_token.is_none() {
            ev.join_token = Some(new_token());
        }
        sqlx::query(
            "UPDATE competitions SET name = ?, format = ?, settings = ?, starts_at = ?, \
             place = ?, signups = ?, min_teams = ?, max_teams = ?, deadline_at = ?, \
             join_token = ? WHERE id = ?",
        )
        .bind(&ev.name)
        .bind(encode(&ev.format)?)
        .bind(encode(&ev.settings)?)
        .bind(ev.schedule.starts_at)
        .bind(&ev.schedule.place)
        .bind(ev.schedule.signups.as_str())
        .bind(ev.schedule.min_teams)
        .bind(ev.schedule.max_teams)
        .bind(ev.schedule.deadline_at)
        .bind(&ev.join_token)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if ev.schedule.starts_at != before.starts_at || ev.schedule.place != before.place {
            notify_signed_up_in(&mut tx, &ev, NoticeKind::EventChanged, now).await?;
        }
        // A new maximum moves teams across the line.
        tell_moves(&mut tx, &ev, lines_before, owner, now).await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// Call off an event that has not started (else `Conflict("event_started")`; one already
    /// cancelled is `Conflict("event_cancelled")`). It closes sign-ups and tells the signed-up
    /// users (not the organiser). It can still be deleted.
    pub async fn cancel_competition(
        &self,
        owner: &str,
        id: &str,
        now: i64,
    ) -> Result<Competition, StoreError> {
        let mut tx = self.begin().await?;
        let mut ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        not_cancelled(&ev)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        cancel_in(&mut tx, &mut ev, now).await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// Delete an event still in `setup` (it has no matches yet) or cancelled, with its teams
    /// and courts.
    pub async fn delete_competition(&self, owner: &str, id: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if !matches!(ev.status, Status::Setup | Status::Cancelled) {
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
        now: i64,
    ) -> Result<EventTeam, StoreError> {
        let name = name.as_deref().map(|n| clean_name(n, "name")).transpose()?;
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        not_cancelled(&ev)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        let before = layout_before(&mut tx, &ev).await?;
        let team = add_team_in(&mut tx, owner, &ev, players, name).await?;
        tell_moves(&mut tx, &ev, before, owner, now).await?;
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
        not_cancelled(&ev)?;
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
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = competition_in(&mut tx, owner, event_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        not_cancelled(&ev)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        let before = layout_before(&mut tx, &ev).await?;
        let done = sqlx::query("DELETE FROM competition_teams WHERE event_id = ? AND id = ?")
            .bind(event_id)
            .bind(team_id)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        tell_moves(&mut tx, &ev, before, owner, now).await?;
        tx.commit().await?;
        Ok(())
    }
}
