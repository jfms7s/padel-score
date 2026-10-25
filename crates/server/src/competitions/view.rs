//! The coordinator's read model of an event: [`EventDetail`], the body of `GET /api/events/{id}`
//! and of each `event` frame on its stream.
//!
//! A match's `state` is `finished` when it has a `match_results` row, else `live` when its log
//! has an effective event, else `waiting`. Only live matches are replayed (for `live`); the
//! rest is plain rows, all read in one transaction so the parts agree.

use std::collections::{HashMap, HashSet};

use padel_scoring::{Score, Team};
use serde::Serialize;
use sqlx::{Row, SqliteConnection};

use super::courts::{courts_in, event_matches_in};
use super::places::{Place, places};
use super::{Audience, Competition, EventTeam, Kind, Status, competition_in, kinds, teams_in};
use crate::commands::view_of;
use crate::hub::CourtPhones;
use crate::results::FinalScore;
use crate::store::{Store, StoreError, decode, events_in, match_by_id_in};

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
    pub teams: Vec<EventTeam>,
    pub courts: Vec<CourtOut>,
    pub matches: Vec<EventMatch>,
    pub places: Vec<Place>,
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
            phones: phones.count(&c.id),
            current: c.current_match_id.clone(),
            queue: queued
                .iter()
                .filter(|(court, ..)| *court == c.id)
                .map(|(.., id)| (*id).to_owned())
                .collect(),
        })
        .collect();

    let places = places(conn, &ev.id).await?;
    let kind_data = kinds::kind_data(ev.kind, conn, &ev, Audience::Coordinator).await?;
    Ok(EventDetail {
        event: EventSummary::new(&ev, teams.len(), courts.len()),
        spectator_token: ev.spectator_token.clone(),
        teams,
        courts: court_out,
        matches,
        places,
        kind_data,
    })
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
