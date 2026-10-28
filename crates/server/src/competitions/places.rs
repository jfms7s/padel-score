//! A team's final place in an event: the pure check that places tile the field, the placings
//! write, and the event's start and finish.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use super::courts::{courts_in, event_matches_in};
use super::{Competition, EventTeam, Status, competition_in, kinds, not_cancelled, teams_in};
use crate::store::{Store, StoreError, Tx, rotate_scorer_token_in};

/// A place, or a tied range of places (`place` to `place_to`, both included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub team_id: String,
    pub place: u32,
    pub place_to: u32,
}

/// Every team appears exactly once and no unknown team does; `1 <= place <= place_to <= n`;
/// the teams sharing a `(place, place_to)` number exactly `place_to - place + 1`; and those
/// ranges, in order, tile `1..=n` with no gap or overlap. Else `InvalidField("places")`.
pub fn validate_places(team_ids: &[String], places: &[Place]) -> Result<(), StoreError> {
    let bad = || StoreError::InvalidField("places");
    let n = u32::try_from(team_ids.len()).map_err(|_| bad())?;
    if places.len() != team_ids.len() {
        return Err(bad());
    }
    let known: HashSet<&str> = team_ids.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut groups: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for p in places {
        if !known.contains(p.team_id.as_str()) || !seen.insert(p.team_id.as_str()) {
            return Err(bad());
        }
        if p.place < 1 || p.place > p.place_to || p.place_to > n {
            return Err(bad());
        }
        *groups.entry((p.place, p.place_to)).or_insert(0) += 1;
    }
    let mut next = 1u32;
    for ((from, to), count) in groups {
        if from != next || count != to - from + 1 {
            return Err(bad());
        }
        next = to + 1;
    }
    if next != n + 1 {
        return Err(bad());
    }
    Ok(())
}

/// Replace the event's placings: two rows per team (both players share the team's place), with
/// the event's kind, the field size and `finished_at = now`. Run again to correct them.
pub async fn write_placings_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    teams: &[EventTeam],
    places: &[Place],
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM placings WHERE event_id = ?")
        .bind(&ev.id)
        .execute(&mut **tx)
        .await?;
    let field_size = i64::try_from(teams.len()).unwrap_or(i64::MAX);
    for team in teams {
        let place = places
            .iter()
            .find(|p| p.team_id == team.id)
            .ok_or(StoreError::InvalidField("places"))?;
        for player in &team.players {
            sqlx::query(
                "INSERT INTO placings (player_id, event_id, event_kind, place, place_to, \
                 field_size, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&player.id)
            .bind(&ev.id)
            .bind(ev.kind.as_str())
            .bind(i64::from(place.place))
            .bind(i64::from(place.place_to))
            .bind(field_size)
            .bind(now)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

/// The event's places as written, one per team (read through the first player), best first.
pub async fn places(conn: &mut SqliteConnection, event_id: &str) -> Result<Vec<Place>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.id, p.place, p.place_to FROM competition_teams t \
         JOIN placings p ON p.event_id = t.event_id AND p.player_id = t.player1_id \
         WHERE t.event_id = ? ORDER BY p.place, t.position, t.id",
    )
    .bind(event_id)
    .fetch_all(conn)
    .await?;
    rows.iter()
        .map(|row| {
            let place: i64 = row.try_get("place")?;
            let place_to: i64 = row.try_get("place_to")?;
            Ok(Place {
                team_id: row.try_get("id")?,
                place: u32::try_from(place).map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                place_to: u32::try_from(place_to).map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(StoreError::from)
}

impl Store {
    /// Move an event from `setup` to `running`: at least 2 teams (`Conflict("too_few_teams")`)
    /// and 1 court (`Conflict("no_courts")`), then the kind's own check and `on_start`.
    pub async fn start_competition(
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
        let teams = teams_in(&mut tx, id).await?;
        if teams.len() < 2 {
            return Err(StoreError::Conflict("too_few_teams"));
        }
        if courts_in(&mut tx, id).await?.is_empty() {
            return Err(StoreError::Conflict("no_courts"));
        }
        kinds::check_start(ev.kind, &mut tx, &ev, &teams).await?;
        sqlx::query("UPDATE competitions SET status = 'running', started_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        ev.status = Status::Running;
        ev.started_at = Some(now);
        kinds::on_start(ev.kind, &mut tx, &ev, now).await?;
        tx.commit().await?;
        Ok(ev)
    }

    /// Finish a running event: write the placings (the kind's computed places, else `places`;
    /// neither is `InvalidField("places")`), rotate the scorer token of every event match and
    /// free every court. Returns the event and the ids of the matches whose tokens were rotated,
    /// so the caller can close their live streams.
    pub async fn finish_competition(
        &self,
        owner: &str,
        id: &str,
        places: Option<Vec<Place>>,
        now: i64,
    ) -> Result<(Competition, Vec<String>), StoreError> {
        let mut tx = self.begin().await?;
        let mut ev = competition_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        not_cancelled(&ev)?;
        if ev.status != Status::Running {
            return Err(StoreError::Conflict("event_not_running"));
        }
        let teams = teams_in(&mut tx, id).await?;
        let places = match kinds::computed_places(ev.kind, &mut tx, &ev).await? {
            Some(computed) => computed,
            None => places.ok_or(StoreError::InvalidField("places"))?,
        };
        let team_ids: Vec<String> = teams.iter().map(|t| t.id.clone()).collect();
        validate_places(&team_ids, &places)?;
        write_placings_in(&mut tx, &ev, &teams, &places, now).await?;
        let mut rotated = Vec::new();
        for m in event_matches_in(&mut tx, id).await? {
            rotate_scorer_token_in(&mut tx, &m.match_id).await?;
            rotated.push(m.match_id);
        }
        sqlx::query("UPDATE courts SET current_match_id = NULL WHERE event_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE competitions SET status = 'finished', finished_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        ev.status = Status::Finished;
        ev.finished_at = Some(now);
        tx.commit().await?;
        Ok((ev, rotated))
    }
}
