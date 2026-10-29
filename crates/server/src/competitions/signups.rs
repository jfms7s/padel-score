//! Sign-ups for a scheduled event: solo and partner sign-ups, automatic pairing, the line past
//! the maximum, and the organiser's split, pair and remove.
//!
//! An `event_signups` row is one entry. It is a **solo** (one account, no partner yet; `held`
//! when the organiser split an automatic pair, so pairing skips it), a **pending** invitation
//! (one account and a `partner_token`), or **complete** (`complete_at` set: two accounts, or one
//! account and a partner typed by name). Complete entries are the teams: ordered by when they
//! became complete, the first `max_teams - hand_added` are in the event and the rest wait in the
//! line. A user is in at most one entry of an event.
//!
//! Every write is one transaction. Whatever moves a team across the maximum line queues
//! `moved_up` or `moved_down` for its members (never for the user whose request caused it); a
//! pair that completes gets `team_complete`. The HTTP layer wakes the push sender afterwards.

mod calendar;
mod organised;

pub(crate) use calendar::{
    calendar_events_in, open_events_in, queue_events_soon_in, waiting_partners_in,
};
pub(crate) use organised::organised_events_in;

use std::collections::HashSet;

use padel_scoring::Format;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use super::schedule::{Schedule, Signups, take_count};
use super::{
    Competition, Kind, MAX_TEAMS, Status, add_team_in, cancel_in, clean_name, competition_by_id_in,
    competition_by_join_token_in, competition_in, not_cancelled,
};
use crate::linking::{auto_entry_in, plain_entry_in};
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{Store, StoreError, Tx};
use crate::tokens::new_token;

/// The most sign-up rows one event keeps.
pub const MAX_SIGNUPS: i64 = 256;
/// The request field a bad partner name is reported under.
const PARTNER_NAME_FIELD: &str = "partner_name";

/// How a user signs up.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum JoinAs {
    Solo,
    /// With a partner: by name when they have no account (the team is complete at once), else
    /// a link for them to open.
    Partner {
        #[serde(default)]
        partner_name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignupPlayer {
    /// The account, when there is one. Left out of the public join page.
    pub user_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignupTeam {
    pub id: String,
    pub players: Vec<SignupPlayer>,
    pub complete_at: Option<i64>,
    pub auto_paired: bool,
    pub held: bool,
    pub partner_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SignupList {
    /// Teams the organiser made by hand: they hold a place of their own.
    pub hand_added: usize,
    /// The complete sign-ups that have a place, in order.
    pub teams: Vec<SignupTeam>,
    /// The complete sign-ups past the maximum, in order.
    pub line: Vec<SignupTeam>,
    /// People waiting for a partner, held ones included.
    pub solos: Vec<SignupTeam>,
    /// People who sent a partner link that is not answered yet.
    pub pending: Vec<SignupTeam>,
}

/// Where a user stands in an event. Places are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MyEntry {
    Solo,
    /// Waiting for the partner to open the link.
    Pending {
        partner_token: String,
    },
    /// In the event: the hand-added teams come first.
    Team {
        place: u32,
    },
    /// In the line past the maximum.
    Line {
        place: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartnerInvite {
    pub event_name: String,
    pub inviter_name: String,
    pub join_token: String,
}

/// What the join page shows about the event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventJoinInfo {
    pub name: String,
    pub kind: Kind,
    pub format: Format,
    pub schedule: Schedule,
    pub status: Status,
    pub organiser_name: String,
    /// Nobody can sign up any more: cancelled, started, past the deadline or taken.
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JoinPage {
    pub event: EventJoinInfo,
    pub list: SignupList,
    pub me: Option<MyEntry>,
}

/// What "Take teams from sign-ups" did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TakeReport {
    /// Sign-ups made into teams of the event.
    pub taken: usize,
    /// Sign-up ids left out because a player is already in a hand-added team (the same account
    /// counts as the same person). They stay on the line.
    pub skipped: Vec<String>,
    /// Complete sign-ups still waiting: the ones past the places, and the skipped.
    pub left_on_line: usize,
}

/// An event the deadline job handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedSignups {
    pub event_id: String,
    pub owner_id: String,
    /// Cancelled for too few teams, rather than just closed.
    pub cancelled: bool,
}

/// A solo waiting for a partner, as [`pairs`] sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solo {
    pub id: String,
    /// When the person signed up.
    pub at: i64,
    /// Row order: who signed up first among those in the same second.
    pub rowid: i64,
    pub held: bool,
}

/// The automatic pairs among `solos`: not held, taken in sign-up order (`at`, then row order): the
/// earliest two, then the next two, and so on. An odd one out waits. Each pair is (earlier,
/// later).
pub fn pairs(solos: &[Solo]) -> Vec<(String, String)> {
    let mut free: Vec<&Solo> = solos.iter().filter(|s| !s.held).collect();
    free.sort_by_key(|s| (s.at, s.rowid));
    free.chunks_exact(2)
        .filter_map(|two| match two {
            [first, second] => Some((first.id.clone(), second.id.clone())),
            _ => None,
        })
        .collect()
}

/// One `event_signups` row with the accounts' display names.
#[derive(Debug, Clone)]
struct Entry {
    id: String,
    /// Creation order, the last tie-break between entries of the same second.
    rowid: i64,
    user1_id: String,
    user1_name: String,
    user1_at: i64,
    user2_id: Option<String>,
    user2_name: Option<String>,
    user2_at: Option<i64>,
    partner_name: Option<String>,
    partner_token: Option<String>,
    auto_paired: bool,
    held: bool,
    complete_at: Option<i64>,
    taken: bool,
}

impl Entry {
    fn from_row(row: &SqliteRow) -> Result<Entry, sqlx::Error> {
        let taken: Option<String> = row.try_get("taken_team_id")?;
        Ok(Entry {
            id: row.try_get("id")?,
            rowid: row.try_get("rowid")?,
            user1_id: row.try_get("user1_id")?,
            user1_name: row.try_get("name1")?,
            user1_at: row.try_get("user1_at")?,
            user2_id: row.try_get("user2_id")?,
            user2_name: row.try_get("name2")?,
            user2_at: row.try_get("user2_at")?,
            partner_name: row.try_get("partner_name")?,
            partner_token: row.try_get("partner_token")?,
            auto_paired: row.try_get("auto_paired")?,
            held: row.try_get("held")?,
            complete_at: row.try_get("complete_at")?,
            taken: taken.is_some(),
        })
    }

    fn is_solo(&self) -> bool {
        self.complete_at.is_none() && self.partner_token.is_none()
    }

    fn has(&self, user: &str) -> bool {
        self.user1_id == user || self.user2_id.as_deref() == Some(user)
    }

    /// The accounts in the entry.
    fn users(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.user1_id.as_str()).chain(self.user2_id.as_deref())
    }

    fn team(&self, show_ids: bool) -> SignupTeam {
        let id_of = |id: &String| show_ids.then(|| id.clone());
        let mut players = vec![SignupPlayer {
            user_id: id_of(&self.user1_id),
            name: self.user1_name.clone(),
        }];
        if let Some(user2) = &self.user2_id {
            players.push(SignupPlayer {
                user_id: id_of(user2),
                name: self.user2_name.clone().unwrap_or_default(),
            });
        } else if let Some(name) = &self.partner_name {
            players.push(SignupPlayer {
                user_id: None,
                name: name.clone(),
            });
        }
        SignupTeam {
            id: self.id.clone(),
            players,
            complete_at: self.complete_at,
            auto_paired: self.auto_paired,
            held: self.held,
            partner_pending: self.partner_token.is_some(),
        }
    }
}

macro_rules! select_entries {
    ($rest:literal) => {
        concat!(
            "SELECT s.id, s.rowid AS rowid, s.event_id, s.user1_id, u1.display_name AS name1, s.user1_at, s.user2_id, \
             u2.display_name AS name2, s.user2_at, s.partner_name, s.partner_token, \
             s.auto_paired, s.held, s.complete_at, s.taken_team_id \
             FROM event_signups s JOIN users u1 ON u1.id = s.user1_id \
             LEFT JOIN users u2 ON u2.id = s.user2_id ",
            $rest
        )
    };
}

async fn entries_in(conn: &mut SqliteConnection, event_id: &str) -> Result<Vec<Entry>, StoreError> {
    let rows = sqlx::query(select_entries!("WHERE s.event_id = ?"))
        .bind(event_id)
        .fetch_all(conn)
        .await?;
    Ok(rows
        .iter()
        .map(Entry::from_row)
        .collect::<Result<Vec<_>, _>>()?)
}

async fn entry_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    entry_id: &str,
) -> Result<Option<Entry>, StoreError> {
    let row = sqlx::query(select_entries!("WHERE s.event_id = ? AND s.id = ?"))
        .bind(event_id)
        .bind(entry_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(Entry::from_row).transpose()?)
}

async fn entry_of_user_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    user: &str,
) -> Result<Option<Entry>, StoreError> {
    let row = sqlx::query(select_entries!(
        "WHERE s.event_id = ?1 AND (s.user1_id = ?2 OR s.user2_id = ?2)"
    ))
    .bind(event_id)
    .bind(user)
    .fetch_optional(conn)
    .await?;
    Ok(row.as_ref().map(Entry::from_row).transpose()?)
}

fn place(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The sign-ups of an event sorted into the lists the page shows.
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    hand_added: usize,
    teams: Vec<Entry>,
    line: Vec<Entry>,
    solos: Vec<Entry>,
    pending: Vec<Entry>,
}

impl Layout {
    /// Complete entries go in order of `(taken first, complete_at, user1_at, rowid)`: the first
    /// `max - hand_added` have a place, the rest are the line.
    fn build(entries: Vec<Entry>, hand_added: usize, max: usize) -> Layout {
        let mut complete = Vec::new();
        let mut solos = Vec::new();
        let mut pending = Vec::new();
        for entry in entries {
            if entry.complete_at.is_some() {
                complete.push(entry);
            } else if entry.partner_token.is_some() {
                pending.push(entry);
            } else {
                solos.push(entry);
            }
        }
        complete.sort_by_key(|e| (!e.taken, e.complete_at, e.user1_at, e.rowid));
        solos.sort_by_key(|e| (e.user1_at, e.rowid));
        pending.sort_by_key(|e| (e.user1_at, e.rowid));
        // Taken entries are teams of the event whatever the maximum says.
        let taken = complete.iter().filter(|e| e.taken).count();
        let room = max
            .saturating_sub(hand_added)
            .max(taken)
            .min(complete.len());
        let line = complete.split_off(room);
        Layout {
            hand_added,
            teams: complete,
            line,
            solos,
            pending,
        }
    }

    /// Solo sign-ups looking for a partner (a pending invitation is not one).
    pub(crate) fn solo_count(&self) -> usize {
        self.solos.len()
    }

    /// Complete teams past the maximum, waiting for a place.
    pub(crate) fn line_count(&self) -> usize {
        self.line.len()
    }

    /// Teams with a place: the hand-added and the complete sign-ups that fit.
    pub(crate) fn team_count(&self) -> usize {
        self.hand_added + self.teams.len()
    }

    fn list(&self, show_ids: bool) -> SignupList {
        let of = |entries: &[Entry]| entries.iter().map(|e| e.team(show_ids)).collect();
        SignupList {
            hand_added: self.hand_added,
            teams: of(&self.teams),
            line: of(&self.line),
            solos: of(&self.solos),
            pending: of(&self.pending),
        }
    }

    fn my_entry(&self, user: &str) -> Option<MyEntry> {
        if let Some(i) = self.teams.iter().position(|e| e.has(user)) {
            return Some(MyEntry::Team {
                place: place(self.hand_added + i + 1),
            });
        }
        if let Some(i) = self.line.iter().position(|e| e.has(user)) {
            return Some(MyEntry::Line {
                place: place(i + 1),
            });
        }
        if self.solos.iter().any(|e| e.has(user)) {
            return Some(MyEntry::Solo);
        }
        self.pending
            .iter()
            .find(|e| e.has(user))
            .and_then(|e| e.partner_token.clone())
            .map(|partner_token| MyEntry::Pending { partner_token })
    }

    /// The entry `user` is in, wherever it sits.
    fn entry_of(&self, user: &str) -> Option<&Entry> {
        self.teams
            .iter()
            .chain(&self.line)
            .chain(&self.solos)
            .chain(&self.pending)
            .find(|e| e.has(user))
    }

    fn team_ids(&self) -> HashSet<&str> {
        self.teams.iter().map(|e| e.id.as_str()).collect()
    }

    fn line_ids(&self) -> HashSet<&str> {
        self.line.iter().map(|e| e.id.as_str()).collect()
    }
}

/// The sort of `event_id`'s sign-ups, with `max_teams` as the maximum.
pub(crate) async fn layout_in(
    conn: &mut SqliteConnection,
    event_id: &str,
    max_teams: Option<u32>,
) -> Result<Layout, StoreError> {
    let entries = entries_in(conn, event_id).await?;
    let hand_added: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM competition_teams t WHERE t.event_id = ? \
         AND NOT EXISTS (SELECT 1 FROM event_signups s WHERE s.taken_team_id = t.id)",
    )
    .bind(event_id)
    .fetch_one(conn)
    .await?;
    let max = max_teams.map_or(MAX_TEAMS, |m| usize::try_from(m).unwrap_or(MAX_TEAMS));
    Ok(Layout::build(
        entries,
        usize::try_from(hand_added).unwrap_or(0),
        max,
    ))
}

fn notice(ev: &Competition, user: &str, kind: NoticeKind, name: Option<&str>) -> Option<Notice> {
    let token = ev.join_token.as_ref()?;
    Some(Notice {
        user_id: user.to_owned(),
        payload: Payload {
            kind,
            url: format!("/j/{token}"),
            title: Some(ev.name.clone()),
            start: ev.schedule.starts_at,
            name: name.map(str::to_owned),
            ..Payload::default()
        },
        dedupe: None,
    })
}

/// `team_complete` for both members, each told the other's name.
fn team_complete(ev: &Competition, a: (&str, &str), b: (&str, &str)) -> Vec<Notice> {
    [
        notice(ev, a.0, NoticeKind::TeamComplete, Some(b.1)),
        notice(ev, b.0, NoticeKind::TeamComplete, Some(a.1)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `moved_up` for entries that went from the line into the teams and `moved_down` for the other
/// way, to their members except `actor`.
fn moves(ev: &Competition, before: &Layout, after: &Layout, actor: &str) -> Vec<Notice> {
    let was_line = before.line_ids();
    let was_team = before.team_ids();
    let moved = |entries: &[Entry], was: &HashSet<&str>, kind: NoticeKind| {
        entries
            .iter()
            .filter(|e| was.contains(e.id.as_str()))
            .flat_map(|e| e.users())
            .filter(|user| *user != actor)
            .filter_map(|user| notice(ev, user, kind, None))
            .collect::<Vec<_>>()
    };
    let mut out = moved(&after.teams, &was_line, NoticeKind::MovedUp);
    out.extend(moved(&after.line, &was_team, NoticeKind::MovedDown));
    out
}

/// Queue the moves between two layouts of `ev`, caused by `actor`.
pub(crate) async fn queue_moves_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    before: &Layout,
    after: &Layout,
    actor: &str,
    now: i64,
) -> Result<(), StoreError> {
    queue_in(tx, &moves(ev, before, after, actor), now).await?;
    Ok(())
}

/// Whether the event takes sign-ups and joins at `now`.
fn check_open(ev: &Competition, now: i64) -> Result<(), StoreError> {
    not_cancelled(ev)?;
    let past_deadline = ev.schedule.deadline_at.is_some_and(|d| now >= d);
    if ev.status != Status::Setup || ev.signups_closed_at.is_some() || past_deadline {
        return Err(StoreError::Conflict("signups_closed"));
    }
    Ok(())
}

/// The event behind a join link; one whose sign-ups are off is as unknown as a wrong link.
async fn event_by_join_token_in(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<Competition, StoreError> {
    match competition_by_join_token_in(conn, token).await? {
        Some(ev) if ev.schedule.signups != Signups::Off => Ok(ev),
        _ => Err(StoreError::NotFound),
    }
}

/// The owner's event, still in setup, for an organiser change: not cancelled, not started.
async fn organiser_event_in(
    conn: &mut SqliteConnection,
    owner: &str,
    event_id: &str,
) -> Result<Competition, StoreError> {
    let ev = competition_in(conn, owner, event_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    not_cancelled(&ev)?;
    if ev.status != Status::Setup {
        return Err(StoreError::Conflict("event_started"));
    }
    Ok(ev)
}

async fn user_name_in(conn: &mut SqliteConnection, user: &str) -> Result<String, StoreError> {
    sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?")
        .bind(user)
        .fetch_optional(conn)
        .await?
        .ok_or(StoreError::NotFound)
}

/// Pair the event's waiting solos (not held) in sign-up order. The later solo's account moves
/// into the earlier row, which becomes a complete automatic pair now, and the later row goes.
/// Returns `team_complete` for every pair made.
async fn auto_pair_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    now: i64,
) -> Result<Vec<Notice>, StoreError> {
    let entries = entries_in(tx, &ev.id).await?;
    let solos: Vec<Solo> = entries
        .iter()
        .filter(|e| e.is_solo())
        .map(|e| Solo {
            id: e.id.clone(),
            at: e.user1_at,
            rowid: e.rowid,
            held: e.held,
        })
        .collect();
    let mut out = Vec::new();
    for (first, second) in pairs(&solos) {
        let find = |id: &str| entries.iter().find(|e| e.id == id);
        let (Some(a), Some(b)) = (find(&first), find(&second)) else {
            continue;
        };
        sqlx::query(
            "UPDATE event_signups SET user2_id = ?, user2_at = ?, auto_paired = 1, held = 0, \
             complete_at = ? WHERE id = ?",
        )
        .bind(&b.user1_id)
        .bind(b.user1_at)
        .bind(now)
        .bind(&a.id)
        .execute(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM event_signups WHERE id = ?")
            .bind(&b.id)
            .execute(&mut **tx)
            .await?;
        out.extend(team_complete(
            ev,
            (&a.user1_id, &a.user1_name),
            (&b.user1_id, &b.user1_name),
        ));
    }
    Ok(out)
}

impl Store {
    /// Close the sign-ups of every event whose deadline has passed (`deadline_at <= now`), for
    /// events in setup with sign-ups on that are not cancelled or closed. With the complete
    /// sign-ups plus the hand-added teams below `min_teams` the event is cancelled and every
    /// signed-up user is told (`event_cancelled`); otherwise `signups_closed_at` is set. Each
    /// event is its own transaction, and one that fails is logged and left for the next tick.
    /// An automatic cancel also tells the organiser (`event_cancelled`): nobody asked for it.
    /// Returns the events handled, for the caller to tell their streams.
    pub async fn close_due_signups(&self, now: i64) -> Result<Vec<ClosedSignups>, StoreError> {
        let due: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM competitions WHERE status = 'setup' AND signups != 'off' \
             AND cancelled_at IS NULL AND signups_closed_at IS NULL \
             AND deadline_at IS NOT NULL AND deadline_at <= ? ORDER BY deadline_at, id",
        )
        .bind(now)
        .fetch_all(self.pool())
        .await?;
        let mut handled = Vec::new();
        for id in due {
            match self.close_signups_of(&id, now).await {
                Ok(Some(closed)) => handled.push(closed),
                Ok(None) => {}
                Err(error) => tracing::error!(%error, event = %id, "closing sign-ups failed"),
            }
        }
        Ok(handled)
    }

    /// [`Store::close_due_signups`] for one event; `None` when it is no longer due.
    async fn close_signups_of(
        &self,
        id: &str,
        now: i64,
    ) -> Result<Option<ClosedSignups>, StoreError> {
        let mut tx = self.begin().await?;
        let Some(mut ev) = competition_by_id_in(&mut tx, id).await? else {
            return Ok(None);
        };
        let due = ev.status == Status::Setup
            && ev.schedule.signups != Signups::Off
            && ev.signups_closed_at.is_none()
            && ev.schedule.deadline_at.is_some_and(|d| d <= now);
        if !due {
            return Ok(None);
        }
        let layout = layout_in(&mut tx, id, ev.schedule.max_teams).await?;
        let teams = layout.hand_added + layout.teams.len() + layout.line.len();
        let cancelled = teams < usize::try_from(ev.schedule.min_teams).unwrap_or(usize::MAX);
        if cancelled {
            cancel_in(&mut tx, &mut ev, now).await?;
            let told = notice(&ev, &ev.owner_user_id, NoticeKind::EventCancelled, None);
            queue_in(&mut tx, &told.into_iter().collect::<Vec<_>>(), now).await?;
        } else {
            sqlx::query("UPDATE competitions SET signups_closed_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(Some(ClosedSignups {
            event_id: ev.id,
            owner_id: ev.owner_user_id,
            cancelled,
        }))
    }

    /// The organiser takes the complete sign-ups as teams of the event (in setup, not
    /// cancelled), now or after the deadline, and closes the sign-ups.
    ///
    /// The event runs with `take_count` teams in all: every team already there counts as
    /// hand-added, and the first complete sign-ups that are not taken yet, in place order, make
    /// up the rest. Each person becomes the organiser's entry linked to their account
    /// (`auto_entry_in`); a partner typed by name becomes a plain entry. A sign-up with a player
    /// already in a team is skipped and the next one tried (see [`TakeReport::skipped`]). The
    /// complete sign-ups left over are told `no_place` once. When there is nothing to take
    /// (`take_count` adds no team), it is `Conflict("nothing_to_take")` and nothing changes.
    pub async fn take_teams(
        &self,
        owner: &str,
        event_id: &str,
        now: i64,
    ) -> Result<TakeReport, StoreError> {
        let mut tx = self.begin().await?;
        let ev = organiser_event_in(&mut tx, owner, event_id).await?;
        if ev.schedule.signups == Signups::Off {
            return Err(StoreError::Conflict("signups_off"));
        }
        let mut waiting: Vec<Entry> = entries_in(&mut tx, event_id)
            .await?
            .into_iter()
            .filter(|e| e.complete_at.is_some() && !e.taken)
            .collect();
        waiting.sort_by_key(|e| (e.complete_at, e.user1_at, e.rowid));
        let hand_added: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM competition_teams WHERE event_id = ?")
                .bind(event_id)
                .fetch_one(&mut *tx)
                .await?;
        let hand_added = usize::try_from(hand_added).unwrap_or(0);
        let max = ev
            .schedule
            .max_teams
            .unwrap_or_else(|| u32::try_from(MAX_TEAMS).unwrap_or(u32::MAX));
        let total = take_count(ev.kind, hand_added, waiting.len(), max);
        let wanted = total.saturating_sub(hand_added);
        if wanted == 0 {
            return Err(StoreError::Conflict("nothing_to_take"));
        }

        let mut taken_ids: HashSet<&str> = HashSet::new();
        let mut skipped = Vec::new();
        for entry in &waiting {
            if taken_ids.len() == wanted {
                break;
            }
            let players = entry_players_in(&mut tx, owner, entry, now).await?;
            match add_team_in(&mut tx, owner, &ev, players, None).await {
                Ok(team) => {
                    sqlx::query("UPDATE event_signups SET taken_team_id = ? WHERE id = ?")
                        .bind(&team.id)
                        .bind(&entry.id)
                        .execute(&mut *tx)
                        .await?;
                    taken_ids.insert(&entry.id);
                }
                Err(StoreError::Conflict("player_in_team")) => skipped.push(entry.id.clone()),
                Err(other) => return Err(other),
            }
        }
        sqlx::query(
            "UPDATE competitions SET signups_closed_at = COALESCE(signups_closed_at, ?) \
             WHERE id = ?",
        )
        .bind(now)
        .bind(event_id)
        .execute(&mut *tx)
        .await?;

        // Whoever is still waiting for a place, and was not set aside for a clash, is told once.
        let no_place: Vec<Notice> = waiting
            .iter()
            .filter(|e| !taken_ids.contains(e.id.as_str()) && !skipped.contains(&e.id))
            .flat_map(Entry::users)
            .filter(|user| *user != owner)
            .filter_map(|user| {
                let mut notice = notice(&ev, user, NoticeKind::NoPlace, None)?;
                notice.dedupe = Some(format!("no-place:{event_id}:{user}"));
                Some(notice)
            })
            .collect();
        queue_in(&mut tx, &no_place, now).await?;
        tx.commit().await?;
        Ok(TakeReport {
            taken: taken_ids.len(),
            left_on_line: waiting.len() - taken_ids.len(),
            skipped,
        })
    }

    /// Sign `user` up through the event's join link. The event must take sign-ups, be in setup
    /// and not closed, and `now` must be before the deadline (`Conflict("signups_closed")`; a
    /// cancelled event is `Conflict("event_cancelled")`). A user already in the event is
    /// `Conflict("already_signed_up")`.
    ///
    /// A solo waits for the next solo and is paired by sign-up time, the pair complete (and told
    /// so) at that moment. A partner typed by name makes a complete team at once; without a
    /// name the user gets a link for the partner.
    pub async fn join_event(
        &self,
        join_token: &str,
        user: &str,
        as_: JoinAs,
        now: i64,
    ) -> Result<MyEntry, StoreError> {
        let partner_name = match &as_ {
            JoinAs::Partner {
                partner_name: Some(raw),
            } => Some(clean_name(raw, PARTNER_NAME_FIELD)?),
            _ => None,
        };
        let mut tx = self.begin().await?;
        let ev = event_by_join_token_in(&mut tx, join_token).await?;
        check_open(&ev, now)?;
        if entry_of_user_in(&mut tx, &ev.id, user).await?.is_some() {
            return Err(StoreError::Conflict("already_signed_up"));
        }
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_signups WHERE event_id = ?")
            .bind(&ev.id)
            .fetch_one(&mut *tx)
            .await?;
        if rows >= MAX_SIGNUPS {
            return Err(StoreError::Conflict("signups_full"));
        }
        let before = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        let (complete_at, token) = match (&as_, &partner_name) {
            (JoinAs::Solo, _) => (None, None),
            (JoinAs::Partner { .. }, Some(_)) => (Some(now), None),
            (JoinAs::Partner { .. }, None) => (None, Some(new_token())),
        };
        sqlx::query(
            "INSERT INTO event_signups (id, event_id, user1_id, user1_at, partner_name, \
             partner_token, complete_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&ev.id)
        .bind(user)
        .bind(now)
        .bind(&partner_name)
        .bind(&token)
        .bind(complete_at)
        .execute(&mut *tx)
        .await?;
        let mut notices = auto_pair_in(&mut tx, &ev, now).await?;
        let after = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        notices.extend(moves(&ev, &before, &after, user));
        queue_in(&mut tx, &notices, now).await?;
        let mine = after
            .my_entry(user)
            .ok_or_else(|| StoreError::Corrupt("sign-up missing after joining".to_owned()))?;
        tx.commit().await?;
        Ok(mine)
    }

    /// Take `user` out of the event. Allowed after the deadline, until the event starts, but not
    /// once the organiser took the team (`Conflict("signup_taken")`); not in the event is
    /// `NotFound`.
    ///
    /// A solo, a pending invitation or a team with a named partner goes entirely. In a team of
    /// two accounts the partner stays as a solo with their own sign-up time, is told
    /// (`partner_left`) and is paired again if another solo waits. Whoever moves across the
    /// maximum line is told.
    pub async fn leave_event(
        &self,
        join_token: &str,
        user: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = event_by_join_token_in(&mut tx, join_token).await?;
        not_cancelled(&ev)?;
        if ev.status != Status::Setup {
            return Err(StoreError::Conflict("event_started"));
        }
        let entry = entry_of_user_in(&mut tx, &ev.id, user)
            .await?
            .ok_or(StoreError::NotFound)?;
        if entry.taken {
            return Err(StoreError::Conflict("signup_taken"));
        }
        let before = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        let mut notices = Vec::new();
        let leaves_first = entry.user1_id == user;
        match (&entry.user2_id, leaves_first) {
            (Some(user2), true) => {
                // The partner takes the first seat, keeping their own sign-up time.
                sqlx::query(
                    "UPDATE event_signups SET user1_id = user2_id, user1_at = user2_at, \
                     user2_id = NULL, user2_at = NULL, complete_at = NULL, auto_paired = 0, \
                     held = 0 WHERE id = ?",
                )
                .bind(&entry.id)
                .execute(&mut *tx)
                .await?;
                notices.extend(notice(
                    &ev,
                    user2,
                    NoticeKind::PartnerLeft,
                    Some(&entry.user1_name),
                ));
            }
            (Some(_), false) => {
                sqlx::query(
                    "UPDATE event_signups SET user2_id = NULL, user2_at = NULL, \
                     complete_at = NULL, auto_paired = 0, held = 0 WHERE id = ?",
                )
                .bind(&entry.id)
                .execute(&mut *tx)
                .await?;
                notices.extend(notice(
                    &ev,
                    &entry.user1_id,
                    NoticeKind::PartnerLeft,
                    entry.user2_name.as_deref(),
                ));
            }
            (None, _) => {
                sqlx::query("DELETE FROM event_signups WHERE id = ?")
                    .bind(&entry.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        notices.extend(auto_pair_in(&mut tx, &ev, now).await?);
        let after = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        notices.extend(moves(&ev, &before, &after, user));
        queue_in(&mut tx, &notices, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The invitation behind a partner link, or `None` for an unknown (or spent) link.
    pub async fn partner_invite(
        &self,
        partner_token: &str,
    ) -> Result<Option<PartnerInvite>, StoreError> {
        let row: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT c.name, u.display_name, c.join_token FROM event_signups s \
             JOIN competitions c ON c.id = s.event_id JOIN users u ON u.id = s.user1_id \
             WHERE s.partner_token = ?",
        )
        .bind(partner_token)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.and_then(|(event_name, inviter_name, join_token)| {
            Some(PartnerInvite {
                event_name,
                inviter_name,
                join_token: join_token?,
            })
        }))
    }

    /// Accept a partner link: `user` becomes the second member and the team is complete now,
    /// with `team_complete` to both. The inviter's own link is `Conflict("own_link")`, someone
    /// already in the event `Conflict("already_signed_up")`, a closed event
    /// `Conflict("signups_closed")` or `Conflict("event_cancelled")`; an unknown link is
    /// `NotFound`.
    pub async fn accept_partner(
        &self,
        partner_token: &str,
        user: &str,
        now: i64,
    ) -> Result<MyEntry, StoreError> {
        let mut tx = self.begin().await?;
        let (ev, entry) = invitation_in(&mut tx, partner_token).await?;
        check_open(&ev, now)?;
        if entry.user1_id == user {
            return Err(StoreError::Conflict("own_link"));
        }
        if entry_of_user_in(&mut tx, &ev.id, user).await?.is_some() {
            return Err(StoreError::Conflict("already_signed_up"));
        }
        let name = user_name_in(&mut tx, user).await?;
        let before = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        sqlx::query(
            "UPDATE event_signups SET user2_id = ?, user2_at = ?, partner_token = NULL, \
             complete_at = ? WHERE id = ?",
        )
        .bind(user)
        .bind(now)
        .bind(now)
        .bind(&entry.id)
        .execute(&mut *tx)
        .await?;
        let after = layout_in(&mut tx, &ev.id, ev.schedule.max_teams).await?;
        let mut notices = team_complete(&ev, (&entry.user1_id, &entry.user1_name), (user, &name));
        notices.extend(moves(&ev, &before, &after, user));
        queue_in(&mut tx, &notices, now).await?;
        let mine = after
            .my_entry(user)
            .ok_or_else(|| StoreError::Corrupt("sign-up missing after accepting".to_owned()))?;
        tx.commit().await?;
        Ok(mine)
    }

    /// Decline a partner link. The link dies and the inviter stays pending with a fresh one to
    /// share. An unknown link is `NotFound`.
    pub async fn decline_partner(
        &self,
        partner_token: &str,
        _user: &str,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let (ev, entry) = invitation_in(&mut tx, partner_token).await?;
        not_cancelled(&ev)?;
        sqlx::query("UPDATE event_signups SET partner_token = ? WHERE id = ?")
            .bind(new_token())
            .bind(&entry.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// What the join page shows `viewer` (none when signed out), or `None` for an unknown link
    /// or one whose sign-ups are off. The list leaves out account ids.
    pub async fn join_page(
        &self,
        join_token: &str,
        viewer: Option<&str>,
        now: i64,
    ) -> Result<Option<JoinPage>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let Some(ev) = competition_by_join_token_in(&mut conn, join_token)
            .await?
            .filter(|ev| ev.schedule.signups != Signups::Off)
        else {
            return Ok(None);
        };
        let layout = layout_in(&mut conn, &ev.id, ev.schedule.max_teams).await?;
        let organiser_name = user_name_in(&mut conn, &ev.owner_user_id).await?;
        let closed = check_open(&ev, now).is_err();
        Ok(Some(JoinPage {
            me: viewer.and_then(|user| layout.my_entry(user)),
            list: layout.list(false),
            event: EventJoinInfo {
                name: ev.name,
                kind: ev.kind,
                format: ev.format,
                schedule: ev.schedule,
                status: ev.status,
                organiser_name,
                closed,
            },
        }))
    }

    /// The event's id and organiser behind a join link, for the layer that tells the desk.
    pub async fn join_link_event(
        &self,
        join_token: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        Ok(
            sqlx::query_as("SELECT id, owner_user_id FROM competitions WHERE join_token = ?")
                .bind(join_token)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// The organiser's view of the sign-ups, or `None` when the event is not theirs.
    pub async fn signup_list(
        &self,
        owner: &str,
        event_id: &str,
    ) -> Result<Option<SignupList>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let Some(ev) = competition_in(&mut conn, owner, event_id).await? else {
            return Ok(None);
        };
        let layout = layout_in(&mut conn, &ev.id, ev.schedule.max_teams).await?;
        Ok(Some(layout.list(true)))
    }

    /// Remove a sign-up (a team, a solo or a pending invitation). One the organiser already
    /// took is `Conflict("signup_taken")`.
    pub async fn remove_signup(
        &self,
        owner: &str,
        event_id: &str,
        entry_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = organiser_event_in(&mut tx, owner, event_id).await?;
        let entry = entry_in(&mut tx, event_id, entry_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if entry.taken {
            return Err(StoreError::Conflict("signup_taken"));
        }
        let before = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        sqlx::query("DELETE FROM event_signups WHERE id = ?")
            .bind(entry_id)
            .execute(&mut *tx)
            .await?;
        let after = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        queue_moves_in(&mut tx, &ev, &before, &after, owner, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Split an automatic pair into two solos. Both are held, so automatic pairing skips them
    /// until the organiser pairs them (`Conflict("not_auto_paired")` for any other entry).
    pub async fn split_signup(
        &self,
        owner: &str,
        event_id: &str,
        entry_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = organiser_event_in(&mut tx, owner, event_id).await?;
        let entry = entry_in(&mut tx, event_id, entry_id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if entry.taken {
            return Err(StoreError::Conflict("signup_taken"));
        }
        let (true, Some(user2), Some(user2_at)) =
            (entry.auto_paired, &entry.user2_id, entry.user2_at)
        else {
            return Err(StoreError::Conflict("not_auto_paired"));
        };
        let before = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        sqlx::query(
            "UPDATE event_signups SET user2_id = NULL, user2_at = NULL, complete_at = NULL, \
             auto_paired = 0, held = 1 WHERE id = ?",
        )
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO event_signups (id, event_id, user1_id, user1_at, held) \
             VALUES (?, ?, ?, ?, 1)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(event_id)
        .bind(user2)
        .bind(user2_at)
        .execute(&mut *tx)
        .await?;
        let after = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        queue_moves_in(&mut tx, &ev, &before, &after, owner, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Pair two solos by hand: the team is complete now, with `team_complete` to both. An entry
    /// that is not a solo is `Conflict("not_solo")`.
    pub async fn pair_signups(
        &self,
        owner: &str,
        event_id: &str,
        a: &str,
        b: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        if a == b {
            return Err(StoreError::InvalidField("entries"));
        }
        let mut tx = self.begin().await?;
        let ev = organiser_event_in(&mut tx, owner, event_id).await?;
        let first = entry_in(&mut tx, event_id, a)
            .await?
            .ok_or(StoreError::NotFound)?;
        let second = entry_in(&mut tx, event_id, b)
            .await?
            .ok_or(StoreError::NotFound)?;
        if !first.is_solo() || !second.is_solo() {
            return Err(StoreError::Conflict("not_solo"));
        }
        let (keep, later) = if (first.user1_at, first.rowid) <= (second.user1_at, second.rowid) {
            (first, second)
        } else {
            (second, first)
        };
        let before = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        sqlx::query(
            "UPDATE event_signups SET user2_id = ?, user2_at = ?, auto_paired = 0, held = 0, \
             complete_at = ? WHERE id = ?",
        )
        .bind(&later.user1_id)
        .bind(later.user1_at)
        .bind(now)
        .bind(&keep.id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM event_signups WHERE id = ?")
            .bind(&later.id)
            .execute(&mut *tx)
            .await?;
        let after = layout_in(&mut tx, event_id, ev.schedule.max_teams).await?;
        let mut notices = team_complete(
            &ev,
            (&keep.user1_id, &keep.user1_name),
            (&later.user1_id, &later.user1_name),
        );
        notices.extend(moves(&ev, &before, &after, owner));
        queue_in(&mut tx, &notices, now).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The organiser's player entries for the two people of a complete sign-up: each account's
/// automatic entry, or for a partner typed by name the plain entry of that name.
async fn entry_players_in(
    tx: &mut Tx<'_>,
    owner: &str,
    entry: &Entry,
    now: i64,
) -> Result<[String; 2], StoreError> {
    let first = auto_entry_in(tx, owner, &entry.user1_id, now).await?;
    let second = match (&entry.user2_id, &entry.partner_name) {
        (Some(user), _) => auto_entry_in(tx, owner, user, now).await?,
        (None, Some(name)) => plain_entry_in(tx, owner, name, now).await?,
        (None, None) => {
            return Err(StoreError::Corrupt(format!(
                "complete sign-up {} has no second player",
                entry.id
            )));
        }
    };
    Ok([first, second])
}

/// The event and pending entry behind a partner link (`NotFound` for an unknown link).
async fn invitation_in(
    conn: &mut SqliteConnection,
    partner_token: &str,
) -> Result<(Competition, Entry), StoreError> {
    let row = sqlx::query(select_entries!("WHERE s.partner_token = ?"))
        .bind(partner_token)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(StoreError::NotFound)?;
    let entry = Entry::from_row(&row)?;
    let event_id: String = row.try_get("event_id")?;
    let ev = competition_by_id_in(conn, &event_id)
        .await?
        .ok_or(StoreError::NotFound)?;
    Ok((ev, entry))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::*;

    fn solo(id: &str, at: i64, held: bool) -> Solo {
        Solo {
            id: id.to_owned(),
            at,
            rowid: 0,
            held,
        }
    }

    #[test]
    fn pairs_in_signup_order() {
        let solos = [
            solo("d", 40, false),
            solo("a", 10, false),
            solo("c", 30, false),
            solo("b", 20, false),
        ];
        assert_eq!(
            pairs(&solos),
            [("a".into(), "b".into()), ("c".into(), "d".into())]
        );
        // The same second falls back to row order, not to the id.
        let mut y = solo("y", 5, false);
        y.rowid = 1;
        let mut x = solo("x", 5, false);
        x.rowid = 2;
        assert_eq!(pairs(&[x.clone(), y.clone()]), [("y".into(), "x".into())]);
        y.rowid = 3;
        assert_eq!(pairs(&[y, x]), [("x".into(), "y".into())]);
    }

    #[test]
    fn odd_solo_waits() {
        let solos = [
            solo("a", 10, false),
            solo("b", 20, false),
            solo("c", 30, false),
        ];
        assert_eq!(pairs(&solos), [("a".into(), "b".into())]);
        assert!(pairs(&solos[..1]).is_empty());
        assert!(pairs(&[]).is_empty());
    }

    #[test]
    fn held_solos_are_skipped() {
        let solos = [
            solo("a", 10, true),
            solo("b", 20, false),
            solo("c", 30, true),
            solo("d", 40, false),
        ];
        assert_eq!(pairs(&solos), [("b".into(), "d".into())]);
        assert!(pairs(&[solo("a", 1, true), solo("b", 2, true)]).is_empty());
    }

    fn entry(id: &str, user: &str, complete_at: Option<i64>, taken: bool) -> Entry {
        Entry {
            id: id.to_owned(),
            rowid: 0,
            user1_id: user.to_owned(),
            user1_name: user.to_owned(),
            user1_at: 1,
            user2_id: None,
            user2_name: None,
            user2_at: None,
            partner_name: complete_at.map(|_| "Named".to_owned()),
            partner_token: None,
            auto_paired: false,
            held: false,
            complete_at,
            taken,
        }
    }

    #[test]
    fn layout_orders_teams_and_cuts_the_line() {
        let entries = vec![
            entry("late", "u3", Some(30), false),
            entry("taken", "u4", Some(99), true),
            entry("early", "u1", Some(10), false),
            entry("tie-a", "u2", Some(20), false),
            entry("tie-b", "u5", Some(20), false),
            entry("solo", "u6", None, false),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, mut e)| {
            e.rowid = i as i64;
            e
        })
        .collect();
        // One place goes to a hand-added team: 4 - 1 = 3 left.
        let layout = Layout::build(entries, 1, 4);
        let ids = |es: &[Entry]| es.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&layout.teams), ["taken", "early", "tie-a"]);
        assert_eq!(ids(&layout.line), ["tie-b", "late"]);
        assert_eq!(ids(&layout.solos), ["solo"]);
        // Places count the hand-added team first; a line place counts from 1.
        assert_eq!(layout.my_entry("u4"), Some(MyEntry::Team { place: 2 }));
        assert_eq!(layout.my_entry("u2"), Some(MyEntry::Team { place: 4 }));
        assert_eq!(layout.my_entry("u3"), Some(MyEntry::Line { place: 2 }));
        assert_eq!(layout.my_entry("u6"), Some(MyEntry::Solo));
        assert_eq!(layout.my_entry("nobody"), None);
    }

    #[test]
    fn a_tie_in_one_second_goes_by_row_order_not_by_id() {
        // Same `complete_at` and `user1_at`; the ids sort the other way round to the rows.
        let mut early = entry("z-first", "u1", Some(5), false);
        early.rowid = 1;
        let mut late = entry("a-second", "u2", Some(5), false);
        late.rowid = 2;
        let layout = Layout::build(vec![late.clone(), early.clone()], 0, 1);
        assert_eq!(layout.teams[0].id, "z-first");
        assert_eq!(layout.line[0].id, "a-second");
        // An earlier sign-up time still wins over a lower row.
        late.user1_at = 0;
        let layout = Layout::build(vec![early, late], 0, 1);
        assert_eq!(layout.teams[0].id, "a-second");
    }

    #[test]
    fn hand_added_teams_can_fill_the_event() {
        let layout = Layout::build(vec![entry("a", "u1", Some(1), false)], 5, 4);
        assert!(layout.teams.is_empty());
        assert_eq!(layout.line.len(), 1);
    }
}
