//! The read models of an event: the coordinator's [`EventDetail`] (`GET /api/events/{id}` and
//! each `event` frame on its stream), the court page's [`CourtState`] (`GET /api/court/{token}`)
//! and the spectators' [`EventView`] (`GET /api/event-view/{token}`), which is the detail with
//! every scorer token and phone count left out.
//!
//! A match's `state` is `finished` when it has a `match_results` row, else `live` when its log
//! has an effective event, else `waiting`. Only live matches are replayed (for `live`); the
//! rest is plain rows, all read in one transaction so the parts agree.

use std::collections::{HashMap, HashSet};

use padel_scoring::{Score, Team};
use serde::Serialize;
use sqlx::{Row, SqliteConnection};

use super::courts::{
    Court, CourtAccess, court_by_token_in, courts_in, event_match_in, event_matches_in,
    next_queued_in,
};
use super::places::{Place, places};
use super::schedule::Schedule;
use super::{
    Audience, Competition, EventTeam, Kind, Status, competition_by_id_in, competition_in, kinds,
    teams_in,
};
use crate::commands::view_of;
use crate::hub::CourtPhones;
use crate::results::FinalScore;
use crate::store::{Store, StoreError, Teams, decode, events_in, match_by_id_in};

/// An event as listed, and at the top of every detail. The spectator token sits beside it in
/// [`EventDetail`], never in a list item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventSummary {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub format: padel_scoring::Format,
    pub settings: serde_json::Value,
    pub status: Status,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub team_count: usize,
    pub court_count: usize,
    pub schedule: Schedule,
    pub cancelled_at: Option<i64>,
}

impl EventSummary {
    pub fn new(ev: &Competition, team_count: usize, court_count: usize) -> EventSummary {
        EventSummary {
            id: ev.id.clone(),
            name: ev.name.clone(),
            kind: ev.kind,
            format: ev.format,
            settings: ev.settings.clone(),
            status: ev.status,
            created_at: ev.created_at,
            started_at: ev.started_at,
            finished_at: ev.finished_at,
            team_count,
            court_count,
            schedule: ev.schedule.clone(),
            cancelled_at: ev.cancelled_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    Waiting,
    Live,
    Finished,
}

/// An event match as the coordinator sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventMatch {
    /// The match id.
    pub id: String,
    pub label: String,
    pub ordinal: i64,
    pub team_a: String,
    pub team_b: String,
    pub court_id: Option<String>,
    pub state: MatchState,
    pub locked: bool,
    pub winner: Option<Team>,
    pub result: Option<FinalScore>,
    /// The replayed score, for a live match only.
    pub live: Option<Score>,
    pub scorer_token: String,
    pub spectator_token: String,
}

/// A court as the coordinator sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtOut {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub scorer_token: String,
    pub spectator_token: String,
    /// Court phones connected now.
    pub phones: usize,
    pub current: Option<String>,
    /// The unfinished matches queued here and current nowhere, in queue order.
    pub queue: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventDetail {
    pub event: EventSummary,
    pub spectator_token: String,
    /// The `/j/{token}` link once sign-ups were turned on.
    pub join_token: Option<String>,
    pub teams: Vec<EventTeam>,
    pub courts: Vec<CourtOut>,
    pub matches: Vec<EventMatch>,
    pub places: Vec<Place>,
    /// The kind works out the places itself: the Finish screen sends none.
    pub places_computed: bool,
    pub kind_data: Option<serde_json::Value>,
}

impl EventDetail {
    pub fn court(&self, court_id: &str) -> Option<&CourtOut> {
        self.courts.iter().find(|c| c.id == court_id)
    }

    pub fn event_match(&self, match_id: &str) -> Option<&EventMatch> {
        self.matches.iter().find(|m| m.id == match_id)
    }
}

/// The owner's event `id` as the coordinator sees it, or `None` when it is not theirs (or does
/// not exist). One read transaction.
pub async fn event_detail(
    store: &Store,
    owner: &str,
    id: &str,
    phones: &CourtPhones,
) -> Result<Option<EventDetail>, StoreError> {
    let mut tx = store.pool().begin().await?;
    let Some(ev) = competition_in(&mut tx, owner, id).await? else {
        return Ok(None);
    };
    let detail = detail_in(&mut tx, ev, phones).await?;
    tx.commit().await?;
    Ok(Some(detail))
}

/// The per-match facts the detail needs beyond the `competition_matches` link.
struct MatchFacts {
    scorer_token: String,
    spectator_token: String,
    locked: bool,
    winner: Option<Team>,
    result: Option<FinalScore>,
    has_events: bool,
}

async fn match_facts_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<HashMap<String, MatchFacts>, StoreError> {
    let rows = sqlx::query(
        "SELECT cm.match_id, m.scorer_token, m.spectator_token, m.result_locked_at, \
         r.winner, r.score, \
         EXISTS (SELECT 1 FROM events e WHERE e.match_id = cm.match_id \
                 AND e.state = 'effective') AS has_events \
         FROM competition_matches cm JOIN matches m ON m.id = cm.match_id \
         LEFT JOIN match_results r ON r.match_id = cm.match_id WHERE cm.event_id = ?",
    )
    .bind(event_id)
    .fetch_all(conn)
    .await?;
    let mut facts = HashMap::new();
    for row in &rows {
        let locked_at: Option<i64> = row.try_get("result_locked_at")?;
        let winner: Option<&str> = row.try_get("winner")?;
        let score: Option<&str> = row.try_get("score")?;
        let winner = match winner {
            Some("a") => Some(Team::A),
            Some("b") => Some(Team::B),
            Some(other) => {
                return Err(StoreError::Db(sqlx::Error::Decode(
                    format!("bad winner {other:?}").into(),
                )));
            }
            None => None,
        };
        facts.insert(
            row.try_get("match_id")?,
            MatchFacts {
                scorer_token: row.try_get("scorer_token")?,
                spectator_token: row.try_get("spectator_token")?,
                locked: locked_at.is_some(),
                winner,
                result: score.map(decode).transpose()?,
                has_events: row.try_get("has_events")?,
            },
        );
    }
    Ok(facts)
}

/// The match's replayed score, read on `conn`.
async fn live_score_in(conn: &mut SqliteConnection, match_id: &str) -> Result<Score, StoreError> {
    let m = match_by_id_in(conn, match_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let events = events_in(conn, match_id).await?;
    let view = view_of(&m, &events)
        .map_err(|e| StoreError::Corrupt(format!("match {match_id} does not replay: {e}")))?;
    Ok(view.score)
}

/// Build the detail of `ev` on `conn` (the caller's read transaction).
pub(crate) async fn detail_in(
    conn: &mut SqliteConnection,
    ev: Competition,
    phones: &CourtPhones,
) -> Result<EventDetail, StoreError> {
    detail_for_in(conn, ev, Some(phones), Audience::Coordinator).await
}

/// The detail of `ev` with the kind's data for `audience`; without `phones` every court counts
/// 0 (a spectator's view drops the count anyway).
async fn detail_for_in(
    conn: &mut SqliteConnection,
    ev: Competition,
    phones: Option<&CourtPhones>,
    audience: Audience,
) -> Result<EventDetail, StoreError> {
    let teams = teams_in(conn, &ev.id).await?;
    let courts = courts_in(conn, &ev.id).await?;
    let links = event_matches_in(conn, &ev.id).await?;
    let mut facts = match_facts_in(conn, &ev.id).await?;

    let mut matches = Vec::with_capacity(links.len());
    for link in &links {
        let f = facts
            .remove(&link.match_id)
            .ok_or_else(|| StoreError::Corrupt(format!("event match {} lost", link.match_id)))?;
        let state = if f.result.is_some() {
            MatchState::Finished
        } else if f.has_events {
            MatchState::Live
        } else {
            MatchState::Waiting
        };
        let live = match state {
            MatchState::Live => Some(live_score_in(conn, &link.match_id).await?),
            _ => None,
        };
        matches.push(EventMatch {
            id: link.match_id.clone(),
            label: link.label.clone(),
            ordinal: link.ordinal,
            team_a: link.team_a_id.clone(),
            team_b: link.team_b_id.clone(),
            court_id: link.court_id.clone(),
            state,
            locked: f.locked,
            winner: f.winner,
            result: f.result,
            live,
            scorer_token: f.scorer_token,
            spectator_token: f.spectator_token,
        });
    }

    let current: HashSet<&str> = courts
        .iter()
        .filter_map(|c| c.current_match_id.as_deref())
        .collect();
    let mut queued: Vec<(&str, i64, i64, &str)> = links
        .iter()
        .zip(&matches)
        .filter(|(link, m)| {
            m.state != MatchState::Finished && !current.contains(link.match_id.as_str())
        })
        .filter_map(|(link, _)| {
            let court = link.court_id.as_deref()?;
            Some((
                court,
                link.court_seq.unwrap_or(i64::MAX),
                link.ordinal,
                link.match_id.as_str(),
            ))
        })
        .collect();
    queued.sort_unstable();
    let court_out: Vec<CourtOut> = courts
        .iter()
        .map(|c| CourtOut {
            id: c.id.clone(),
            name: c.name.clone(),
            position: c.position,
            scorer_token: c.scorer_token.clone(),
            spectator_token: c.spectator_token.clone(),
            phones: phones.map_or(0, |p| p.count(&c.id)),
            current: c.current_match_id.clone(),
            queue: queued
                .iter()
                .filter(|(court, ..)| *court == c.id)
                .map(|(.., id)| (*id).to_owned())
                .collect(),
        })
        .collect();

    let places = places(conn, &ev.id).await?;
    let kind_data = kinds::kind_data(ev.kind, conn, &ev, audience).await?;
    Ok(EventDetail {
        event: EventSummary::new(&ev, teams.len(), courts.len()),
        spectator_token: ev.spectator_token.clone(),
        join_token: ev.join_token.clone(),
        teams,
        courts: court_out,
        matches,
        places,
        places_computed: kinds::computes_places(ev.kind),
        kind_data,
    })
}

/// The event as a court page shows it at the top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtEvent {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub status: Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtRef {
    pub id: String,
    pub name: String,
}

/// A match as a court page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtMatch {
    /// The match id.
    pub id: String,
    pub label: String,
    pub teams: Teams,
    pub spectator_token: String,
    /// The match's scorer token: only for the current match, and only when the page came in
    /// by the court's scorer token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorer_token: Option<String>,
}

/// `GET /api/court/{token}`, and each `court` frame on its stream.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CourtState {
    pub event: CourtEvent,
    pub court: CourtRef,
    /// The court's current match; a finished one stays until the court is advanced.
    pub current: Option<CourtMatch>,
    /// The match the court moves on to: the lowest `court_seq` unfinished queued match.
    pub next: Option<CourtMatch>,
    pub kind_data: Option<serde_json::Value>,
}

async fn court_match_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    match_id: &str,
    with_scorer_token: bool,
) -> Result<CourtMatch, StoreError> {
    let link = event_match_in(conn, event_id, match_id)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("court match {match_id} has no link")))?;
    let m = match_by_id_in(conn, match_id)
        .await?
        .ok_or_else(|| StoreError::Corrupt(format!("court match {match_id} lost")))?;
    Ok(CourtMatch {
        id: m.id,
        label: link.label,
        teams: m.teams,
        spectator_token: m.spectator_token,
        scorer_token: with_scorer_token.then_some(m.scorer_token),
    })
}

/// Build the court page's state on `conn` (the caller's read transaction).
async fn court_state_in(
    conn: &mut SqliteConnection,
    ev: &Competition,
    court: &Court,
    access: CourtAccess,
) -> Result<CourtState, StoreError> {
    let current = match &court.current_match_id {
        Some(id) => Some(court_match_in(conn, &ev.id, id, access == CourtAccess::Scorer).await?),
        None => None,
    };
    let next = match next_queued_in(conn, &court.id).await? {
        Some(id) => Some(court_match_in(conn, &ev.id, &id, false).await?),
        None => None,
    };
    let audience = Audience::Court {
        court_id: court.id.clone(),
    };
    let kind_data = kinds::kind_data(ev.kind, conn, ev, audience).await?;
    Ok(CourtState {
        event: CourtEvent {
            id: ev.id.clone(),
            name: ev.name.clone(),
            kind: ev.kind,
            status: ev.status,
        },
        court: CourtRef {
            id: court.id.clone(),
            name: court.name.clone(),
        },
        current,
        next,
        kind_data,
    })
}

/// The court page's state for `court` of `ev`, as the token `access` sees it. One read
/// transaction; the court's current match is the one in `court`.
pub async fn court_state(
    store: &Store,
    ev: &Competition,
    court: &Court,
    access: CourtAccess,
) -> Result<CourtState, StoreError> {
    let mut tx = store.pool().begin().await?;
    let state = court_state_in(&mut tx, ev, court, access).await?;
    tx.commit().await?;
    Ok(state)
}

/// A court found by one of its tokens, with its page's state.
#[derive(Debug, Clone)]
pub struct CourtLookup {
    pub event: Competition,
    pub court: Court,
    pub access: CourtAccess,
    pub state: CourtState,
}

/// The court whose scorer or spectator token is `token` and its page's state, read in one
/// transaction so the court and its matches agree; `None` for an unknown token.
pub async fn court_state_by_token(
    store: &Store,
    token: &str,
) -> Result<Option<CourtLookup>, StoreError> {
    let mut tx = store.pool().begin().await?;
    let Some((event, court, access)) = court_by_token_in(&mut tx, token).await? else {
        return Ok(None);
    };
    let state = court_state_in(&mut tx, &event, &court, access).await?;
    tx.commit().await?;
    Ok(Some(CourtLookup {
        event,
        court,
        access,
        state,
    }))
}

/// A court as a spectator of the event sees it: [`CourtOut`] without its scorer token or the
/// phone count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtView {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub spectator_token: String,
    pub current: Option<String>,
    pub queue: Vec<String>,
}

/// An event match as a spectator sees it: [`EventMatch`] without its scorer token.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventMatchView {
    pub id: String,
    pub label: String,
    pub ordinal: i64,
    pub team_a: String,
    pub team_b: String,
    pub court_id: Option<String>,
    pub state: MatchState,
    pub locked: bool,
    pub winner: Option<Team>,
    pub result: Option<FinalScore>,
    pub live: Option<Score>,
    pub spectator_token: String,
}

/// `GET /api/event-view/{token}`, and each `event` frame on its stream: [`EventDetail`] with
/// no scorer token anywhere and no phone counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventView {
    pub event: EventSummary,
    pub spectator_token: String,
    pub teams: Vec<EventTeam>,
    pub courts: Vec<CourtView>,
    pub matches: Vec<EventMatchView>,
    pub places: Vec<Place>,
    pub kind_data: Option<serde_json::Value>,
}

impl From<EventDetail> for EventView {
    fn from(d: EventDetail) -> EventView {
        EventView {
            event: d.event,
            spectator_token: d.spectator_token,
            teams: d.teams,
            courts: d
                .courts
                .into_iter()
                .map(|c| CourtView {
                    id: c.id,
                    name: c.name,
                    position: c.position,
                    spectator_token: c.spectator_token,
                    current: c.current,
                    queue: c.queue,
                })
                .collect(),
            matches: d
                .matches
                .into_iter()
                .map(|m| EventMatchView {
                    id: m.id,
                    label: m.label,
                    ordinal: m.ordinal,
                    team_a: m.team_a,
                    team_b: m.team_b,
                    court_id: m.court_id,
                    state: m.state,
                    locked: m.locked,
                    winner: m.winner,
                    result: m.result,
                    live: m.live,
                    spectator_token: m.spectator_token,
                })
                .collect(),
            places: d.places,
            kind_data: d.kind_data,
        }
    }
}

/// The id of the event whose spectator token is `token`, or `None`.
pub async fn event_id_by_spectator_token(
    store: &Store,
    token: &str,
) -> Result<Option<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT id FROM competitions WHERE spectator_token = ?")
            .bind(token)
            .fetch_optional(store.pool())
            .await?,
    )
}

/// The event whose spectator token is `spectator_token`, as its spectators see it, or `None`.
/// One read transaction.
pub async fn event_view(
    store: &Store,
    spectator_token: &str,
) -> Result<Option<EventView>, StoreError> {
    let mut tx = store.pool().begin().await?;
    let id: Option<String> =
        sqlx::query_scalar("SELECT id FROM competitions WHERE spectator_token = ?")
            .bind(spectator_token)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(id) = id else {
        return Ok(None);
    };
    let ev = competition_by_id_in(&mut tx, &id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let detail = detail_for_in(&mut tx, ev, None, Audience::Spectator).await?;
    tx.commit().await?;
    Ok(Some(detail.into()))
}

/// The event an event match belongs to, or `None` for any other match: one indexed lookup, for
/// telling an event's streams that one of its matches changed.
pub async fn event_id_of_match(
    store: &Store,
    match_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT event_id FROM competition_matches WHERE match_id = ?")
            .bind(match_id)
            .fetch_optional(store.pool())
            .await?,
    )
}

/// `"Court 1 · Sunday cup"` for a match that is current on a court, else `None`: the live
/// list's context line.
pub async fn court_context(store: &Store, match_id: &str) -> Result<Option<String>, StoreError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT c.name, e.name FROM courts c JOIN competitions e ON e.id = c.event_id \
         WHERE c.current_match_id = ?",
    )
    .bind(match_id)
    .fetch_optional(store.pool())
    .await?;
    Ok(row.map(|(court, event)| format!("{court} · {event}")))
}
