//! One calendar over everything a user plays or organises: the games on their boards, the
//! events they organise or signed up for, and the matches they planned or hold a place in. Also
//! what waits for their answer, the boards they are on, what is open for them to join, and the
//! 24h reminders the tick queues.
//!
//! The queries live next to the code that owns the tables ([`crate::boards`],
//! [`crate::competitions::signups`], [`crate::scheduled`]); this module is the shape of the
//! answer and the assembly. Tokens in the answer are the viewer's own or ones they are entitled
//! to (their boards, the events and matches they are in or organise, and public listings); the
//! route is wrapped in `accounts::secret_headers`.

use serde::Serialize;

use crate::boards::GameState;
use crate::competitions::{Kind, Status};
use crate::scheduled::ScheduledState;
use crate::store::{Store, StoreError};

/// Seconds in a day: the reminder window, and the unit of the `from` clamp.
pub const DAY_SECS: i64 = 86_400;
/// `items` is capped at this many.
pub const MAX_ITEMS: usize = 200;
/// `open` is capped at this many.
pub const MAX_OPEN: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Organiser,
    Player,
    Line,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CalendarItem {
    Game {
        board_token: String,
        board_title: String,
        game_id: String,
        start: i64,
        end: i64,
        board_place: Option<String>,
        players: Vec<String>,
        /// The caller's index in `players`.
        me: usize,
        state: GameState,
    },
    Event {
        /// Set for the organiser only.
        event_id: Option<String>,
        join_token: Option<String>,
        name: String,
        event_kind: Kind,
        starts_at: i64,
        place: Option<String>,
        role: Role,
        team: Option<Vec<String>>,
        place_in_line: Option<u32>,
        status: Status,
        /// Teams with a place, hand-added ones included.
        teams: usize,
        max_teams: Option<u32>,
    },
    Match {
        join_token: String,
        owner_name: String,
        starts_at: i64,
        place: Option<String>,
        role: Role,
        /// The four places in order; `None` is an open place.
        players: Vec<Option<String>>,
        state: ScheduledState,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitingItem {
    Swap {
        request_id: String,
        board_token: String,
        from_name: String,
        /// The viewer's own game.
        my_game: (i64, i64),
        /// The asker's game.
        their_game: (i64, i64),
    },
    Time {
        request_id: String,
        board_token: String,
        from_name: String,
        game: (i64, i64),
        new: (i64, i64),
    },
    /// The viewer's own open partner invitation, with the link to resend.
    Partner {
        join_token: String,
        event_name: String,
        partner_token: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardStrip {
    pub token: String,
    pub title: String,
    pub starts_at: i64,
    pub ends_at: i64,
    /// The viewer's games on it that have not ended.
    pub my_games: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenItem {
    Event {
        join_token: String,
        name: String,
        event_kind: Kind,
        starts_at: i64,
        place: Option<String>,
        teams: usize,
        max_teams: u32,
    },
    Match {
        join_token: String,
        owner_name: String,
        starts_at: i64,
        place: Option<String>,
        open_places: usize,
    },
}

impl OpenItem {
    fn starts_at(&self) -> i64 {
        match self {
            OpenItem::Event { starts_at, .. } | OpenItem::Match { starts_at, .. } => *starts_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calendar {
    pub waiting: Vec<WaitingItem>,
    pub items: Vec<CalendarItem>,
    pub boards: Vec<BoardStrip>,
    pub open: Vec<OpenItem>,
}

/// The order of kinds among items that start together: game, event, match.
fn kind_order(item: &CalendarItem) -> u8 {
    match item {
        CalendarItem::Game { .. } => 0,
        CalendarItem::Event { .. } => 1,
        CalendarItem::Match { .. } => 2,
    }
}

fn start_of(item: &CalendarItem) -> i64 {
    match item {
        CalendarItem::Game { start, .. } => *start,
        CalendarItem::Event { starts_at, .. } | CalendarItem::Match { starts_at, .. } => *starts_at,
    }
}

/// Items sorted by `(start, kind order, id)` and capped at [`MAX_ITEMS`]. Each comes with the id
/// of the game, event or match behind it.
fn sorted(mut items: Vec<(String, CalendarItem)>) -> Vec<CalendarItem> {
    items.sort_by(|(a_id, a), (b_id, b)| {
        (start_of(a), kind_order(a), a_id).cmp(&(start_of(b), kind_order(b), b_id))
    });
    items
        .into_iter()
        .map(|(_, item)| item)
        .take(MAX_ITEMS)
        .collect()
}

impl Store {
    /// The user's calendar as of `now`.
    ///
    /// - `items`: anything starting at or after `from`, by start: their games, the dated events
    ///   they organise or are signed up for (cancelled ones too, with their status) and the
    ///   matches they own, hold a place in or wait in line for.
    /// - `waiting`: swap and time requests they must answer (stale ones left out) and their own
    ///   open partner invitations.
    /// - `boards`: their boards that are not over.
    /// - `open`: public events with sign-ups open and public matches with a place free that
    ///   they are not part of, by start, at most [`MAX_OPEN`].
    pub async fn calendar(&self, user: &str, from: i64, now: i64) -> Result<Calendar, StoreError> {
        // One read transaction, so every list sees the same snapshot.
        let mut tx = self.pool().begin().await?;
        let mut items = crate::boards::calendar_games_in(&mut tx, user, from, now).await?;
        items.extend(
            crate::competitions::signups::calendar_events_in(&mut tx, user, from, now).await?,
        );
        items.extend(crate::scheduled::calendar_matches_in(&mut tx, user, from, now).await?);
        let (mut waiting, boards) =
            crate::boards::waiting_and_strips_in(&mut tx, user, now).await?;
        waiting
            .extend(crate::competitions::signups::waiting_partners_in(&mut tx, user, now).await?);
        let mut open = crate::competitions::signups::open_events_in(&mut tx, user, now).await?;
        open.extend(crate::scheduled::open_matches_in(&mut tx, user, now).await?);
        open.sort_by_key(OpenItem::starts_at);
        open.truncate(MAX_OPEN);
        Ok(Calendar {
            waiting,
            items: sorted(items),
            boards,
            open,
        })
    }

    /// Queue the 24h reminders due as of `now`; returns how many notices were queued.
    ///
    /// - `short_game`: a board game with three players that has not been played and starts
    ///   within the next day goes to every member of the board who is in no game overlapping it.
    /// - `match_short`: a scheduled match with three places filled starting within the day goes
    ///   to the people holding them.
    /// - `event_soon`: a dated event in setup starting within the day goes to the people in it
    ///   (teams, not the line, solos or pending invitations).
    ///
    /// Every key is per user (`short:{game}:{start}:{user}`,
    /// `match-short:{match}:{start}:{user}`, `event-soon:{event}:{start}:{user}`) because the outbox's `dedupe` is unique across
    /// all rows, and carry the start, so a moved game, edited match or rescheduled event reminds again.
    /// The caller wakes the push sender.
    pub async fn queue_reminders(&self, now: i64) -> Result<usize, StoreError> {
        let mut tx = self.begin().await?;
        let mut queued = crate::boards::queue_short_games_in(&mut tx, now).await?;
        queued += crate::scheduled::queue_short_matches_in(&mut tx, now).await?;
        queued += crate::competitions::signups::queue_events_soon_in(&mut tx, now).await?;
        tx.commit().await?;
        Ok(queued)
    }
}
