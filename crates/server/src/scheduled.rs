//! Scheduled matches: a match planned for later from New match ("Play later"). It has four
//! places that people take through its join link, a line for when it is full, and becomes a
//! live match when the owner starts it (see [`crate::starting`]).
//!
//! A filled place holds an account (`user_id`: someone who joined, or a pre-filled entry linked
//! to one), an entry of the owner's (`player_id`), or both; an open place has no row. Every write
//! is one `BEGIN IMMEDIATE` transaction and only queues notices ([`queue_in`]); the HTTP layer
//! wakes the push sender. No notice goes to the user whose request caused it.

mod calendar;

pub(crate) use calendar::{calendar_matches_in, open_matches_in, queue_short_matches_in};

use padel_scoring::Format;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use crate::boards::formation::FILL_CUTOFF_SECS;
use crate::linking::auto_entry_in;
use crate::players::linked_player_in;
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::starting::{StartPlayer, StartPlayers, StartedFrom, check_window};
use crate::store::{Store, StoreError, Tx, decode, encode, normalize_place, present};
use crate::tokens::new_token;

/// How long a match lasts for the "past" mark, the longest game.
pub const MATCH_SECS: i64 = 5400;
pub const MAX_OPEN_SCHEDULED_PER_OWNER: usize = 50;
pub const MAX_LINE: usize = 50;
const PLACES: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Listing {
    Private,
    Public,
}

impl Listing {
    fn as_str(self) -> &'static str {
        match self {
            Listing::Private => "private",
            Listing::Public => "public",
        }
    }

    fn parse(text: &str) -> Result<Listing, StoreError> {
        match text {
            "private" => Ok(Listing::Private),
            "public" => Ok(Listing::Public),
            other => Err(StoreError::Corrupt(format!("scheduled listing {other:?}"))),
        }
    }
}

/// A new scheduled match. Each place is one of the owner's player ids, or `null`.
#[derive(Debug, Clone, Deserialize)]
pub struct NewScheduled {
    pub starts_at: i64,
    #[serde(default)]
    pub place: Option<String>,
    pub listing: Listing,
    pub format: Format,
    #[serde(default)]
    pub places: [Option<String>; 4],
}

/// An owner's edit. `place` is `None` when absent, `Some(None)` for an explicit `null`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScheduledChange {
    #[serde(default)]
    pub starts_at: Option<i64>,
    #[serde(default, deserialize_with = "present")]
    pub place: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledState {
    Open,
    Full,
    Started,
    Cancelled,
    /// [`MATCH_SECS`] after the start without a match being started.
    Past,
}

/// A place or a spot in the line. `slot` is the place number, or the position in the line.
/// `user_id` is left out unless the viewer is the owner or it is the viewer's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaceOut {
    pub slot: u8,
    pub name: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduledMe {
    pub owner: bool,
    pub slot: Option<u8>,
    pub in_line: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScheduledView {
    pub id: String,
    pub join_token: String,
    pub owner_name: String,
    pub starts_at: i64,
    pub place: Option<String>,
    pub listing: Listing,
    pub format: Format,
    /// Always four, in slot order.
    pub places: Vec<PlaceOut>,
    pub line: Vec<PlaceOut>,
    pub state: ScheduledState,
    pub match_spectator_token: Option<String>,
    /// `None` for a viewer who is signed out.
    pub me: Option<ScheduledMe>,
}

/// One `scheduled_matches` row with its owner's name and the started match's spectator token.
struct Sched {
    id: String,
    owner_id: String,
    owner_name: String,
    join_token: String,
    starts_at: i64,
    place: Option<String>,
    listing: Listing,
    format: Format,
    cancelled: bool,
    started: bool,
    spectator_token: Option<String>,
}

macro_rules! select_scheduled {
    ($rest:literal) => {
        concat!(
            "SELECT s.id, s.owner_user_id, u.display_name, s.join_token, s.starts_at, s.place, \
             s.listing, s.format, s.cancelled_at IS NOT NULL, s.started_at IS NOT NULL, \
             m.spectator_token \
             FROM scheduled_matches s JOIN users u ON u.id = s.owner_user_id \
             LEFT JOIN matches m ON m.id = s.match_id ",
            $rest
        )
    };
}

async fn row_in(
    conn: &mut SqliteConnection,
    by: &str,
    value: &str,
) -> Result<Option<Sched>, StoreError> {
    let sql = match by {
        "token" => select_scheduled!("WHERE s.join_token = ?"),
        _ => select_scheduled!("WHERE s.id = ?"),
    };
    let Some(r) = sqlx::query(sql)
        .bind(value)
        .fetch_optional(&mut *conn)
        .await?
    else {
        return Ok(None);
    };
    let format: String = r.try_get(7)?;
    let listing: String = r.try_get(6)?;
    Ok(Some(Sched {
        id: r.try_get(0)?,
        owner_id: r.try_get(1)?,
        owner_name: r.try_get(2)?,
        join_token: r.try_get(3)?,
        starts_at: r.try_get(4)?,
        place: r.try_get(5)?,
        listing: Listing::parse(&listing)?,
        format: decode(&format)?,
        cancelled: r.try_get(8)?,
        started: r.try_get(9)?,
        spectator_token: r.try_get(10)?,
    }))
}

async fn by_token_in(conn: &mut SqliteConnection, token: &str) -> Result<Sched, StoreError> {
    row_in(conn, "token", token)
        .await?
        .ok_or(StoreError::NotFound)
}

/// The owner's match; anyone else's, or an unknown one, is `NotFound`.
async fn owned_in(conn: &mut SqliteConnection, owner: &str, id: &str) -> Result<Sched, StoreError> {
    row_in(conn, "id", id)
        .await?
        .filter(|row| row.owner_id == owner)
        .ok_or(StoreError::NotFound)
}

/// A match that can no longer be joined or changed.
fn check_open(row: &Sched, now: i64) -> Result<(), StoreError> {
    if row.cancelled || row.started || row.starts_at.saturating_add(MATCH_SECS) <= now {
        return Err(StoreError::Conflict("match_closed"));
    }
    Ok(())
}

/// One filled place: its slot, the account behind it if any, and the name to show.
struct Filled {
    slot: u8,
    user_id: Option<String>,
    player_id: Option<String>,
    name: String,
}

async fn filled_in(conn: &mut SqliteConnection, id: &str) -> Result<Vec<Filled>, StoreError> {
    let rows = sqlx::query(
        "SELECT p.slot, COALESCE(p.user_id, pl.linked_user_id), p.player_id, \
         COALESCE(pl.name, u.display_name) \
         FROM scheduled_places p \
         LEFT JOIN players pl ON pl.id = p.player_id LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.scheduled_id = ? ORDER BY p.slot",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|r| {
            let slot: i64 = r.try_get(0)?;
            Ok(Filled {
                slot: u8::try_from(slot).map_err(|_| StoreError::Corrupt("place slot".into()))?,
                user_id: r.try_get(1)?,
                player_id: r.try_get(2)?,
                name: r.try_get::<Option<String>, _>(3)?.unwrap_or_default(),
            })
        })
        .collect()
}

/// The line in order: `(user id, display name)`.
async fn line_in(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Vec<(String, String)>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT l.user_id, u.display_name FROM scheduled_line l JOIN users u ON u.id = l.user_id \
         WHERE l.scheduled_id = ? ORDER BY l.joined_at, l.rowid",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?)
}

/// The place `user` holds, directly or through an entry of the owner's linked to them.
async fn slot_of_in(
    conn: &mut SqliteConnection,
    id: &str,
    user: &str,
) -> Result<Option<i64>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT p.slot FROM scheduled_places p LEFT JOIN players pl ON pl.id = p.player_id \
         WHERE p.scheduled_id = ?1 AND (p.user_id = ?2 OR pl.linked_user_id = ?2) \
         ORDER BY p.slot LIMIT 1",
    )
    .bind(id)
    .bind(user)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn in_line_in(conn: &mut SqliteConnection, id: &str, user: &str) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM scheduled_line WHERE scheduled_id = ? AND user_id = ?)",
    )
    .bind(id)
    .bind(user)
    .fetch_one(&mut *conn)
    .await?)
}

async fn open_slot_in(conn: &mut SqliteConnection, id: &str) -> Result<Option<u8>, StoreError> {
    let taken: Vec<i64> =
        sqlx::query_scalar("SELECT slot FROM scheduled_places WHERE scheduled_id = ?")
            .bind(id)
            .fetch_all(&mut *conn)
            .await?;
    Ok((0..PLACES).find(|slot| !taken.contains(&i64::from(*slot))))
}

/// A match's state as of `now`: cancelled, then started, then past, else full or open by how many
/// places are filled.
fn state_of(
    cancelled: bool,
    started: bool,
    starts_at: i64,
    filled: usize,
    now: i64,
) -> ScheduledState {
    if cancelled {
        ScheduledState::Cancelled
    } else if started {
        ScheduledState::Started
    } else if starts_at.saturating_add(MATCH_SECS) <= now {
        ScheduledState::Past
    } else if filled == usize::from(PLACES) {
        ScheduledState::Full
    } else {
        ScheduledState::Open
    }
}

async fn view_in(
    conn: &mut SqliteConnection,
    row: Sched,
    viewer: Option<&str>,
    now: i64,
) -> Result<ScheduledView, StoreError> {
    let filled = filled_in(conn, &row.id).await?;
    let line = line_in(conn, &row.id).await?;
    let is_owner = viewer == Some(row.owner_id.as_str());
    let shown = |user: Option<String>| user.filter(|u| is_owner || viewer == Some(u.as_str()));
    let places: Vec<PlaceOut> = (0..PLACES)
        .map(|slot| {
            let found = filled.iter().find(|f| f.slot == slot);
            PlaceOut {
                slot,
                name: found.map(|f| f.name.clone()),
                user_id: found.and_then(|f| shown(f.user_id.clone())),
            }
        })
        .collect();
    let me = viewer.map(|v| ScheduledMe {
        owner: is_owner,
        slot: filled
            .iter()
            .find(|f| f.user_id.as_deref() == Some(v))
            .map(|f| f.slot),
        in_line: line.iter().any(|(u, _)| u == v),
    });
    let state = state_of(row.cancelled, row.started, row.starts_at, filled.len(), now);
    let line = (0u8..)
        .zip(line)
        .map(|(slot, (user, name))| PlaceOut {
            slot,
            name: Some(name),
            user_id: shown(Some(user)),
        })
        .collect();
    Ok(ScheduledView {
        id: row.id,
        join_token: row.join_token,
        owner_name: row.owner_name,
        starts_at: row.starts_at,
        place: row.place,
        listing: row.listing,
        format: row.format,
        places,
        line,
        state,
        match_spectator_token: row.spectator_token,
        me,
    })
}

/// A notice about `row` for `user`.
fn notice(kind: NoticeKind, row: &Sched, user: String) -> Notice {
    Notice {
        user_id: user,
        payload: Payload {
            kind,
            url: format!("/j/{}", row.join_token),
            name: Some(row.owner_name.clone()),
            start: Some(row.starts_at),
            ..Payload::default()
        },
        dedupe: None,
    }
}

/// Everyone with a place or in the line, once each, except `skip`.
async fn everyone_in(
    conn: &mut SqliteConnection,
    row: &Sched,
    skip: &str,
) -> Result<Vec<String>, StoreError> {
    let mut users: Vec<String> = filled_in(conn, &row.id)
        .await?
        .into_iter()
        .filter_map(|f| f.user_id)
        .chain(line_in(conn, &row.id).await?.into_iter().map(|(u, _)| u))
        .filter(|u| u != skip)
        .collect();
    users.sort();
    users.dedup();
    Ok(users)
}

async fn tell_in(
    tx: &mut Tx<'_>,
    kind: NoticeKind,
    row: &Sched,
    users: Vec<String>,
    now: i64,
) -> Result<(), StoreError> {
    let notices: Vec<Notice> = users
        .into_iter()
        .map(|user| notice(kind, row, user))
        .collect();
    queue_in(tx, &notices, now).await?;
    Ok(())
}

/// Give the freed `slot` to the first in line, unless the match is inside the last hour: then
/// it stays open for anyone to take by hand. `actor` is never told.
async fn fill_from_line_in(
    tx: &mut Tx<'_>,
    row: &Sched,
    slot: u8,
    actor: &str,
    now: i64,
) -> Result<(), StoreError> {
    if row.starts_at.saturating_sub(now) < FILL_CUTOFF_SECS {
        return Ok(());
    }
    let queue: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM scheduled_line WHERE scheduled_id = ? ORDER BY joined_at, rowid",
    )
    .bind(&row.id)
    .fetch_all(&mut **tx)
    .await?;
    for user in queue {
        // Someone who got a place another way (an entry linked to them after they joined the
        // line) is dropped from the line, never given a second place.
        if slot_of_in(tx, &row.id, &user).await?.is_some() {
            sqlx::query("DELETE FROM scheduled_line WHERE scheduled_id = ? AND user_id = ?")
                .bind(&row.id)
                .bind(&user)
                .execute(&mut **tx)
                .await?;
            continue;
        }
        take_place_in(tx, &row.id, &user, slot, now).await?;
        if user != actor {
            tell_in(tx, NoticeKind::PlaceOpened, row, vec![user], now).await?;
        }
        break;
    }
    Ok(())
}

/// Move `user` out of the line (if there) into the open `slot`.
async fn take_place_in(
    tx: &mut Tx<'_>,
    id: &str,
    user: &str,
    slot: u8,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM scheduled_line WHERE scheduled_id = ? AND user_id = ?")
        .bind(id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO scheduled_places (scheduled_id, slot, user_id, joined_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(i64::from(slot))
    .bind(user)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Store {
    /// Make a scheduled match for `owner`. The start is in the future, the place and format are
    /// valid and the pre-filled entries are the owner's active ones, each once (and each
    /// account once). An entry linked to an account copies it into its place. The owner may
    /// have [`MAX_OPEN_SCHEDULED_PER_OWNER`] matches that are not cancelled, started or past
    /// (`Conflict("too_many_scheduled")`).
    pub async fn create_scheduled(
        &self,
        owner: &str,
        new: NewScheduled,
        now: i64,
    ) -> Result<ScheduledView, StoreError> {
        if new.starts_at <= now {
            return Err(StoreError::InvalidField("starts_at"));
        }
        new.format.validate()?;
        let place = normalize_place(new.place.as_deref())?;
        let mut tx = self.begin().await?;
        let open: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scheduled_matches WHERE owner_user_id = ? \
             AND cancelled_at IS NULL AND started_at IS NULL AND starts_at + ? > ?",
        )
        .bind(owner)
        .bind(MATCH_SECS)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if usize::try_from(open).unwrap_or(usize::MAX) >= MAX_OPEN_SCHEDULED_PER_OWNER {
            return Err(StoreError::Conflict("too_many_scheduled"));
        }
        let mut picked: Vec<(u8, String, Option<String>)> = Vec::new();
        for (slot, id) in (0u8..).zip(&new.places) {
            let Some(id) = id else { continue };
            let (player, linked) = linked_player_in(&mut tx, owner, id)
                .await?
                .filter(|(p, _)| !p.archived)
                .ok_or(StoreError::InvalidPlayers("unknown player"))?;
            let twice = picked.iter().any(|(_, other, user)| {
                *other == player.id || (linked.is_some() && *user == linked)
            });
            if twice {
                return Err(StoreError::InvalidPlayers("same player twice"));
            }
            picked.push((slot, player.id, linked));
        }
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO scheduled_matches (id, owner_user_id, join_token, starts_at, place, \
             listing, format, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(owner)
        .bind(new_token())
        .bind(new.starts_at)
        .bind(&place)
        .bind(new.listing.as_str())
        .bind(encode(&new.format)?)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for (slot, player_id, user_id) in &picked {
            sqlx::query(
                "INSERT INTO scheduled_places (scheduled_id, slot, user_id, player_id, joined_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(i64::from(*slot))
            .bind(user_id)
            .bind(player_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        let row = owned_in(&mut tx, owner, &id).await?;
        let view = view_in(&mut tx, row, Some(owner), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// The match behind a join link, as `viewer` sees it, or `None` for an unknown link.
    pub async fn scheduled_by_token(
        &self,
        token: &str,
        viewer: Option<&str>,
        now: i64,
    ) -> Result<Option<ScheduledView>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        let Some(row) = row_in(&mut conn, "token", token).await? else {
            return Ok(None);
        };
        Ok(Some(view_in(&mut conn, row, viewer, now).await?))
    }

    /// Take the lowest open place, or a spot in the line when it is full
    /// (`Conflict("line_full")` past [`MAX_LINE`]). A match that is cancelled, started or past
    /// is `Conflict("match_closed")`. Someone who already holds a place (also through an entry
    /// linked to them) or a spot in the line is `Conflict("already_in")`, except that a line
    /// member moves into an open place: inside the last hour places stay open for anyone to
    /// take by hand.
    pub async fn join_scheduled(
        &self,
        token: &str,
        user: &str,
        now: i64,
    ) -> Result<ScheduledView, StoreError> {
        let mut tx = self.begin().await?;
        let row = by_token_in(&mut tx, token).await?;
        check_open(&row, now)?;
        let open = open_slot_in(&mut tx, &row.id).await?;
        if slot_of_in(&mut tx, &row.id, user).await?.is_some() {
            return Err(StoreError::Conflict("already_in"));
        }
        let queued = in_line_in(&mut tx, &row.id, user).await?;
        match (open, queued) {
            (Some(slot), _) => take_place_in(&mut tx, &row.id, user, slot, now).await?,
            (None, true) => return Err(StoreError::Conflict("already_in")),
            (None, false) => {
                let waiting = line_in(&mut tx, &row.id).await?.len();
                if waiting >= MAX_LINE {
                    return Err(StoreError::Conflict("line_full"));
                }
                sqlx::query(
                    "INSERT INTO scheduled_line (scheduled_id, user_id, joined_at) VALUES (?, ?, ?)",
                )
                .bind(&row.id)
                .bind(user)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
        }
        let view = view_in(&mut tx, row, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Leave the match: free the place (or the spot in the line). The first in line takes a
    /// freed place when the start is at least [`FILL_CUTOFF_SECS`] away. `NotFound` when
    /// `user` was not in.
    pub async fn leave_scheduled(
        &self,
        token: &str,
        user: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let row = by_token_in(&mut tx, token).await?;
        check_open(&row, now)?;
        if let Some(slot) = slot_of_in(&mut tx, &row.id, user).await? {
            sqlx::query("DELETE FROM scheduled_places WHERE scheduled_id = ? AND slot = ?")
                .bind(&row.id)
                .bind(slot)
                .execute(&mut *tx)
                .await?;
            let slot = u8::try_from(slot).map_err(|_| StoreError::Corrupt("place slot".into()))?;
            fill_from_line_in(&mut tx, &row, slot, user, now).await?;
        } else {
            let done =
                sqlx::query("DELETE FROM scheduled_line WHERE scheduled_id = ? AND user_id = ?")
                    .bind(&row.id)
                    .bind(user)
                    .execute(&mut *tx)
                    .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound);
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Change the start or the place of the owner's match, telling the players and the line
    /// (not the owner) when something changed. A start must be in the future.
    pub async fn update_scheduled(
        &self,
        owner: &str,
        id: &str,
        change: ScheduledChange,
        now: i64,
    ) -> Result<ScheduledView, StoreError> {
        let mut tx = self.begin().await?;
        let mut row = owned_in(&mut tx, owner, id).await?;
        check_open(&row, now)?;
        let mut changed = false;
        if let Some(starts_at) = change.starts_at {
            if starts_at <= now {
                return Err(StoreError::InvalidField("starts_at"));
            }
            changed |= starts_at != row.starts_at;
            row.starts_at = starts_at;
        }
        if let Some(place) = change.place {
            let place = normalize_place(place.as_deref())?;
            changed |= place != row.place;
            row.place = place;
        }
        if changed {
            sqlx::query("UPDATE scheduled_matches SET starts_at = ?, place = ? WHERE id = ?")
                .bind(row.starts_at)
                .bind(&row.place)
                .bind(&row.id)
                .execute(&mut *tx)
                .await?;
            let who = everyone_in(&mut tx, &row, owner).await?;
            tell_in(&mut tx, NoticeKind::MatchChanged, &row, who, now).await?;
        }
        // A start moved out of the last hour reopens the line's claim on open places.
        for slot in 0..PLACES {
            if open_slot_in(&mut tx, &row.id).await? == Some(slot) {
                fill_from_line_in(&mut tx, &row, slot, owner, now).await?;
            }
        }
        let view = view_in(&mut tx, row, Some(owner), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Take the person in `slot` out of the match and tell them. The first in line takes the
    /// place as when someone leaves. An empty or unknown slot is `NotFound`.
    pub async fn remove_from_scheduled(
        &self,
        owner: &str,
        id: &str,
        slot: u8,
        now: i64,
    ) -> Result<ScheduledView, StoreError> {
        let mut tx = self.begin().await?;
        let row = owned_in(&mut tx, owner, id).await?;
        check_open(&row, now)?;
        let found = filled_in(&mut tx, &row.id)
            .await?
            .into_iter()
            .find(|f| f.slot == slot)
            .ok_or(StoreError::NotFound)?;
        sqlx::query("DELETE FROM scheduled_places WHERE scheduled_id = ? AND slot = ?")
            .bind(&row.id)
            .bind(i64::from(slot))
            .execute(&mut *tx)
            .await?;
        if let Some(user) = found.user_id.filter(|u| u != owner) {
            tell_in(&mut tx, NoticeKind::RemovedFromMatch, &row, vec![user], now).await?;
        }
        fill_from_line_in(&mut tx, &row, slot, owner, now).await?;
        let view = view_in(&mut tx, row, Some(owner), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Cancel the owner's match and tell the players and the line.
    pub async fn cancel_scheduled(
        &self,
        owner: &str,
        id: &str,
        now: i64,
    ) -> Result<ScheduledView, StoreError> {
        let mut tx = self.begin().await?;
        let mut row = owned_in(&mut tx, owner, id).await?;
        check_open(&row, now)?;
        sqlx::query("UPDATE scheduled_matches SET cancelled_at = ? WHERE id = ?")
            .bind(now)
            .bind(&row.id)
            .execute(&mut *tx)
            .await?;
        let who = everyone_in(&mut tx, &row, owner).await?;
        tell_in(&mut tx, NoticeKind::MatchCancelled, &row, who, now).await?;
        row.cancelled = true;
        let view = view_in(&mut tx, row, Some(owner), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// The four players for New match, in place order (the first two are side A), as the
    /// owner's entries: a pre-filled entry is kept, an account becomes an automatic link. The
    /// match must be the owner's (`NotFound`), not cancelled (`match_closed`), not started
    /// (`already_started`), in its window (`not_yet`, `game_over`) and have four filled places
    /// (`match_short`).
    pub async fn start_scheduled(
        &self,
        owner: &str,
        id: &str,
        now: i64,
    ) -> Result<StartPlayers, StoreError> {
        let mut tx = self.begin().await?;
        let row = owned_in(&mut tx, owner, id).await?;
        check_startable(&row, now)?;
        let filled = filled_in(&mut tx, &row.id).await?;
        if filled.len() < usize::from(PLACES) {
            return Err(StoreError::Conflict("match_short"));
        }
        let mut players: Vec<StartPlayer> = Vec::with_capacity(filled.len());
        let mut accounts: Vec<String> = Vec::new();
        for place in filled {
            let player_id = match (place.player_id, place.user_id) {
                (Some(player), _) => player,
                (None, Some(user)) => auto_entry_in(&mut tx, owner, &user, now).await?,
                (None, None) => return Err(StoreError::Corrupt("empty place".into())),
            };
            let (name, linked): (String, Option<String>) =
                sqlx::query_as("SELECT name, linked_user_id FROM players WHERE id = ?")
                    .bind(&player_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let account_twice = linked.as_ref().is_some_and(|u| accounts.contains(u));
            if account_twice || players.iter().any(|p| p.player_id == player_id) {
                return Err(StoreError::Conflict("same_player_twice"));
            }
            accounts.extend(linked);
            players.push(StartPlayer { player_id, name });
        }
        tx.commit().await?;
        Ok(StartPlayers {
            players,
            format: Some(row.format),
            from: StartedFrom::Scheduled { id: row.id },
        })
    }
}

/// Not cancelled, not started and `now` in the start window.
fn check_startable(row: &Sched, now: i64) -> Result<(), StoreError> {
    if row.cancelled {
        return Err(StoreError::Conflict("match_closed"));
    }
    if row.started {
        return Err(StoreError::Conflict("already_started"));
    }
    check_window(row.starts_at, row.starts_at.saturating_add(MATCH_SECS), now)
}

/// The checks of [`crate::starting::create_match_from`] for a scheduled match: it is the
/// owner's, not cancelled, not started, in its window and has four filled places.
pub(crate) async fn check_startable_in(
    conn: &mut SqliteConnection,
    id: &str,
    owner: &str,
    now: i64,
) -> Result<(), StoreError> {
    let row = owned_in(conn, owner, id).await?;
    check_startable(&row, now)?;
    if filled_in(conn, &row.id).await?.len() < usize::from(PLACES) {
        return Err(StoreError::Conflict("match_short"));
    }
    Ok(())
}

/// Mark the match started by `match_id`, once.
pub(crate) async fn mark_started_in(
    tx: &mut Tx<'_>,
    id: &str,
    match_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    let done = sqlx::query(
        "UPDATE scheduled_matches SET match_id = ?, started_at = ? \
         WHERE id = ? AND started_at IS NULL",
    )
    .bind(match_id)
    .bind(now)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    if done.rows_affected() != 1 {
        return Err(StoreError::Conflict("already_started"));
    }
    Ok(())
}
