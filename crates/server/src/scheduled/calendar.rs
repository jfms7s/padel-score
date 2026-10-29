//! The match side of the calendar: the matches a user owns or has a place or a spot in the line
//! for, the public matches with a place free, and the short-match reminder.

use sqlx::SqliteConnection;

use super::{PLACES, filled_in, state_of};
use crate::calendar::{CalendarItem, MAX_ITEMS, MAX_OPEN, OpenItem, Role};
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{StoreError, Tx};

/// `(id, owner, owner name, join token, starts at, place, cancelled, started)`.
type MatchRow = (
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    bool,
    bool,
);

/// The matches starting at or after `from` that `user` owns, holds a place in (directly or
/// through an entry of the owner's linked to them) or waits in line for, cancelled ones too.
/// Each comes with its id for sorting.
pub(crate) async fn calendar_matches_in(
    conn: &mut SqliteConnection,
    user: &str,
    from: i64,
    now: i64,
) -> Result<Vec<(String, CalendarItem)>, StoreError> {
    let rows: Vec<MatchRow> = sqlx::query_as(
        "SELECT s.id, s.owner_user_id, u.display_name, s.join_token, s.starts_at, \
         s.place, s.cancelled_at IS NOT NULL, s.started_at IS NOT NULL \
         FROM scheduled_matches s JOIN users u ON u.id = s.owner_user_id \
         WHERE s.starts_at >= ?1 AND (s.owner_user_id = ?2 \
           OR EXISTS (SELECT 1 FROM scheduled_places p LEFT JOIN players pl ON pl.id = p.player_id \
             WHERE p.scheduled_id = s.id AND (p.user_id = ?2 OR pl.linked_user_id = ?2)) \
           OR EXISTS (SELECT 1 FROM scheduled_line l \
             WHERE l.scheduled_id = s.id AND l.user_id = ?2)) \
         ORDER BY s.starts_at, s.id LIMIT ?3",
    )
    .bind(from)
    .bind(user)
    .bind(i64::try_from(MAX_ITEMS).unwrap_or(i64::MAX))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (id, owner, owner_name, join_token, starts_at, place, cancelled, started) in rows {
        let filled = filled_in(conn, &id).await?;
        let role = if owner == user {
            Role::Organiser
        } else if filled.iter().any(|f| f.user_id.as_deref() == Some(user)) {
            Role::Player
        } else {
            Role::Line
        };
        let players = (0..PLACES)
            .map(|slot| {
                filled
                    .iter()
                    .find(|f| f.slot == slot)
                    .map(|f| f.name.clone())
            })
            .collect();
        out.push((
            id,
            CalendarItem::Match {
                join_token,
                owner_name,
                starts_at,
                place,
                role,
                players,
                state: state_of(cancelled, started, starts_at, filled.len(), now),
            },
        ));
    }
    Ok(out)
}

/// Public matches with a place free that `user` is not part of (not the owner, no place, not in
/// the line), not cancelled or started, starting ahead.
pub(crate) async fn open_matches_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<Vec<OpenItem>, StoreError> {
    let rows: Vec<(String, String, i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT s.join_token, u.display_name, s.starts_at, s.place, \
           (SELECT COUNT(*) FROM scheduled_places p WHERE p.scheduled_id = s.id) AS filled \
         FROM scheduled_matches s JOIN users u ON u.id = s.owner_user_id \
         WHERE s.listing = 'public' AND s.cancelled_at IS NULL AND s.started_at IS NULL \
         AND s.starts_at > ?1 AND s.owner_user_id != ?2 AND filled < ?4 \
         AND NOT EXISTS (SELECT 1 FROM scheduled_places p \
           LEFT JOIN players pl ON pl.id = p.player_id \
           WHERE p.scheduled_id = s.id AND (p.user_id = ?2 OR pl.linked_user_id = ?2)) \
         AND NOT EXISTS (SELECT 1 FROM scheduled_line l \
           WHERE l.scheduled_id = s.id AND l.user_id = ?2) \
         ORDER BY s.starts_at, s.id LIMIT ?3",
    )
    .bind(now)
    .bind(user)
    .bind(i64::try_from(MAX_OPEN).unwrap_or(i64::MAX))
    .bind(i64::from(PLACES))
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(join_token, owner_name, starts_at, place, filled)| OpenItem::Match {
                join_token,
                owner_name,
                starts_at,
                place,
                open_places: usize::from(PLACES)
                    .saturating_sub(usize::try_from(filled).unwrap_or(0)),
            },
        )
        .collect())
}

/// `match_short` for matches with exactly three places filled that start within the next day
/// (not cancelled or started), to the accounts holding them. Returns how many notices were
/// queued.
pub(crate) async fn queue_short_matches_in(tx: &mut Tx<'_>, now: i64) -> Result<usize, StoreError> {
    let due: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT s.id, s.join_token, u.display_name, s.starts_at FROM scheduled_matches s \
         JOIN users u ON u.id = s.owner_user_id \
         WHERE s.cancelled_at IS NULL AND s.started_at IS NULL \
         AND s.starts_at > ?1 AND s.starts_at <= ?1 + ?2 \
         AND (SELECT COUNT(*) FROM scheduled_places p WHERE p.scheduled_id = s.id) = 3 \
         ORDER BY s.starts_at, s.id",
    )
    .bind(now)
    .bind(crate::calendar::DAY_SECS)
    .fetch_all(&mut **tx)
    .await?;
    let mut queued = 0;
    for (id, token, owner_name, starts_at) in due {
        let mut players: Vec<String> = filled_in(tx, &id)
            .await?
            .into_iter()
            .filter_map(|f| f.user_id)
            .collect();
        players.sort();
        players.dedup();
        let notices: Vec<Notice> = players
            .into_iter()
            .map(|user| Notice {
                dedupe: Some(format!("match-short:{id}:{starts_at}:{user}")),
                user_id: user,
                payload: Payload {
                    kind: NoticeKind::MatchShort,
                    url: format!("/j/{token}"),
                    name: Some(owner_name.clone()),
                    start: Some(starts_at),
                    ..Payload::default()
                },
            })
            .collect();
        queued += queue_in(tx, &notices, now).await?;
    }
    Ok(queued)
}
