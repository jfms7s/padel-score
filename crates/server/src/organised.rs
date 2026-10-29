//! Everything a user runs, in one read: the events they own, the scheduled matches they own and
//! the boards they created. The Organise page lists it.
//!
//! The queries live next to the code that owns the tables ([`crate::competitions::signups`],
//! [`crate::scheduled`], [`crate::boards`]); this module is the shape of the answer and the
//! assembly. The answer carries join and board tokens, but only of things the caller owns, and
//! the route is wrapped in `accounts::secret_headers`.

use serde::Serialize;

use crate::competitions::view::EventSummary;
use crate::scheduled::ScheduledState;
use crate::store::{Store, StoreError};

/// `matches` is capped at this many.
pub const MAX_MATCHES: usize = 200;
/// `boards` is capped at this many.
pub const MAX_BOARDS: usize = 100;

#[derive(Debug, Serialize)]
pub struct OrganisedEvent {
    #[serde(flatten)]
    pub summary: EventSummary,
    pub join_token: Option<String>,
    /// Teams with a place (`summary.team_count` counts only hand-added and taken teams, so it
    /// reads 0 while people are still signing up).
    pub signed_teams: usize,
    pub solo_count: usize,
    pub line_count: usize,
}

#[derive(Debug, Serialize)]
pub struct OrganisedMatch {
    pub id: String,
    pub join_token: String,
    pub starts_at: i64,
    pub place: Option<String>,
    pub state: ScheduledState,
    /// The name in each place by slot; `None` is an open place.
    pub places: [Option<String>; 4],
    pub line_count: usize,
}

#[derive(Debug, Serialize)]
pub struct OrganisedBoard {
    pub token: String,
    pub title: String,
    pub place: Option<String>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub over: bool,
    /// Everyone on the board now, guests included.
    pub member_count: usize,
    pub game_count: usize,
}

#[derive(Debug, Serialize)]
pub struct Organised {
    pub events: Vec<OrganisedEvent>,
    pub matches: Vec<OrganisedMatch>,
    pub boards: Vec<OrganisedBoard>,
}

impl Store {
    /// What `user` runs as of `now`: all their events (newest first), their latest
    /// [`MAX_MATCHES`] scheduled matches and latest [`MAX_BOARDS`] boards, by start descending.
    /// A board stays listed after its creator leaves it.
    pub async fn organised(&self, user: &str, now: i64) -> Result<Organised, StoreError> {
        // One read transaction, so every list sees the same snapshot.
        let mut tx = self.pool().begin().await?;
        Ok(Organised {
            events: crate::competitions::signups::organised_events_in(&mut tx, user).await?,
            matches: crate::scheduled::organised_matches_in(&mut tx, user, now).await?,
            boards: crate::boards::organised_boards_in(&mut tx, user, now).await?,
        })
    }
}
