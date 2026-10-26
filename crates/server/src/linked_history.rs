//! A player's combined history across every coordinator: phase 5's history join with
//! `players.linked_user_id = ?` in place of `players.id = ?`. Nothing is copied or moved; an
//! unlink drops the entry from the next read. Who may see which entry is decided by
//! [`crate::privacy`], never here.

use padel_scoring::Team;
use serde::Serialize;
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::results::{
    Cursor, EventContext, FinalScore, Outcome, Record, entry_from_row, event_joins,
    history_columns, outcome_of, parse_team, placing_from_row, push_page, record_columns,
    record_from_row, split_page,
};
use crate::store::{Store, StoreError, Teams};

/// One match in a combined history. The full form shows names, the coordinator and the match
/// link; the redacted form (`redacted: true`) has none of those keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedEntry {
    pub match_id: String,
    pub finished_at: i64,
    pub outcome: Outcome,
    pub side: Team,
    pub score: FinalScore,
    pub redacted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teams: Option<Teams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinator_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectator_token: Option<String>,
    /// The event the match was played in, for an event match only; a redacted entry keeps only
    /// its kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<LinkedEvent>,
    /// The match's coordinator, read by [`crate::privacy::redact`]; never served.
    #[serde(skip)]
    pub owner_user_id: String,
}

/// A linked entry's event: phase 6's [`EventContext`] in full form, the kind alone when
/// redacted (no id, name or label).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl From<EventContext> for LinkedEvent {
    fn from(event: EventContext) -> LinkedEvent {
        LinkedEvent {
            id: Some(event.id),
            name: Some(event.name),
            kind: event.kind,
            label: Some(event.label),
        }
    }
}

impl LinkedEvent {
    /// The kind alone, for a redacted entry.
    pub fn redacted(self) -> LinkedEvent {
        LinkedEvent {
            id: None,
            name: None,
            kind: self.kind,
            label: None,
        }
    }
}

/// A placing in a combined history: phase 5's `Placing`, with the event's name only for a
/// viewer allowed to see it ([`crate::privacy::redact_placing`] clears it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedPlacing {
    pub event_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_name: Option<String>,
    pub event_kind: String,
    pub place: i64,
    pub place_to: i64,
    pub field_size: i64,
    pub finished_at: i64,
    /// The event's coordinator, read by [`crate::privacy::redact_placing`]; never served.
    #[serde(skip)]
    pub owner_user_id: String,
}

/// The start of the derived table `mp` of the matches `user_id` is linked into, once per
/// (match, side): it ends where the user id goes, then [`LINKED_MATCHES_END`].
macro_rules! linked_matches {
    () => {
        "(SELECT DISTINCT mp.match_id, mp.team FROM players p \
          JOIN match_players mp ON mp.player_id = p.id WHERE p.linked_user_id = "
    };
}
const LINKED_MATCHES_END: &str = ") mp";

impl Store {
    /// The record over every finished match of the entries linked to `user_id`.
    pub async fn linked_record(&self, user_id: &str) -> Result<Record, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            record_columns!(),
            " FROM ",
            linked_matches!(),
            "?) mp JOIN match_results r ON r.match_id = mp.match_id"
        ))
        .bind(user_id)
        .fetch_one(self.pool())
        .await?;
        // SUM over no rows is NULL.
        if row.try_get::<Option<i64>, _>("wins")?.is_none() {
            return Ok(Record::default());
        }
        record_from_row(&row)
    }

    /// One page of the finished matches of the entries linked to `user_id`, newest first, in
    /// full form (each carries its coordinator's name); one query, joined to `users`.
    pub async fn linked_history(
        &self,
        user_id: &str,
        before: Option<&Cursor>,
        limit: u32,
    ) -> Result<(Vec<LinkedEntry>, Option<Cursor>), StoreError> {
        let limit = limit.max(1);
        let mut q = QueryBuilder::<Sqlite>::new(concat!(
            "SELECT ",
            history_columns!(),
            ", mp.team AS side, r.owner_user_id, u.display_name AS coordinator_name FROM ",
            linked_matches!()
        ));
        q.push_bind(user_id.to_owned())
            .push(LINKED_MATCHES_END)
            .push(
                " JOIN match_results r ON r.match_id = mp.match_id \
                 JOIN matches m ON m.id = r.match_id \
                 JOIN users u ON u.id = r.owner_user_id",
            )
            .push(event_joins!())
            .push(" WHERE 1 = 1");
        push_page(&mut q, before, limit);
        let rows = q.build().fetch_all(self.pool()).await?;
        let items = rows
            .iter()
            .map(|row| {
                let entry = entry_from_row(row)?;
                let side = parse_team(row.try_get::<&str, _>("side")?)?;
                Ok(LinkedEntry {
                    match_id: entry.match_id,
                    finished_at: entry.finished_at,
                    outcome: outcome_of(entry.winner, side),
                    side,
                    score: entry.score,
                    redacted: false,
                    teams: Some(entry.teams),
                    coordinator_name: Some(row.try_get("coordinator_name")?),
                    spectator_token: Some(entry.spectator_token),
                    event: entry.event.map(LinkedEvent::from),
                    owner_user_id: row.try_get("owner_user_id")?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(split_page(items, limit, |e| Cursor {
            finished_at: e.finished_at,
            match_id: e.match_id.clone(),
        }))
    }

    /// The placings of the entries linked to `user_id`, newest first.
    pub async fn linked_placings(&self, user_id: &str) -> Result<Vec<LinkedPlacing>, StoreError> {
        let rows = sqlx::query(
            "SELECT pl.event_id, c.name AS event_name, c.owner_user_id, pl.event_kind, pl.place, pl.place_to, \
             pl.field_size, pl.finished_at FROM placings pl JOIN players p ON p.id = pl.player_id \
             JOIN competitions c ON c.id = pl.event_id \
             WHERE p.linked_user_id = ? ORDER BY pl.finished_at DESC, pl.event_id DESC",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                let p = placing_from_row(row)?;
                Ok(LinkedPlacing {
                    event_id: p.event_id,
                    event_name: Some(p.event_name),
                    event_kind: p.event_kind,
                    place: p.place,
                    place_to: p.place_to,
                    field_size: p.field_size,
                    finished_at: p.finished_at,
                    owner_user_id: row.try_get("owner_user_id")?,
                })
            })
            .collect()
    }
}
