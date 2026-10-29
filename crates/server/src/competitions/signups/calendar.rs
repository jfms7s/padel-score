//! The event side of the calendar: the events a user organises or signed up for, their own open
//! partner invitations, the public events open to join, and the 24h reminder.

use sqlx::{Row, SqliteConnection};

use super::{Entry, MyEntry, layout_in};
use crate::calendar::{CalendarItem, MAX_ITEMS, MAX_OPEN, OpenItem, Role, WaitingItem};
use crate::competitions::{Kind, MAX_TEAMS, competition_by_id_in};
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{StoreError, Tx};

/// The dated events starting at or after `from` that `user` organises or is signed up for
/// (cancelled ones too), each with its id for sorting.
///
/// The role is `Organiser` for the owner (one item even when also signed up), `Player` for an
/// entry with a place, `Line` for a line team (with its place in the line) and for a solo or a
/// pending entry (with none).
pub(crate) async fn calendar_events_in(
    conn: &mut SqliteConnection,
    user: &str,
    from: i64,
    _now: i64,
) -> Result<Vec<(String, CalendarItem)>, StoreError> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT c.id FROM competitions c WHERE c.starts_at IS NOT NULL AND c.starts_at >= ?1 \
         AND (c.owner_user_id = ?2 OR EXISTS ( \
           SELECT 1 FROM event_signups s \
           WHERE s.event_id = c.id AND (s.user1_id = ?2 OR s.user2_id = ?2))) \
         ORDER BY c.starts_at, c.id LIMIT ?3",
    )
    .bind(from)
    .bind(user)
    .bind(i64::try_from(MAX_ITEMS).unwrap_or(i64::MAX))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for id in ids {
        let Some(ev) = competition_by_id_in(conn, &id).await? else {
            continue;
        };
        let Some(starts_at) = ev.schedule.starts_at else {
            continue;
        };
        let layout = layout_in(conn, &ev.id, ev.schedule.max_teams).await?;
        let mine = layout.my_entry(user);
        let team = layout.entry_of(user).map(|e: &Entry| {
            e.team(false)
                .players
                .into_iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
        });
        let place_in_line = match mine {
            Some(MyEntry::Line { place }) => Some(place),
            _ => None,
        };
        let role = if ev.owner_user_id == user {
            Role::Organiser
        } else if matches!(mine, Some(MyEntry::Team { .. })) {
            Role::Player
        } else {
            Role::Line
        };
        out.push((
            ev.id.clone(),
            CalendarItem::Event {
                event_id: (role == Role::Organiser).then(|| ev.id.clone()),
                join_token: ev.join_token.clone(),
                name: ev.name.clone(),
                event_kind: ev.kind,
                starts_at,
                place: ev.schedule.place.clone(),
                role,
                team,
                place_in_line,
                status: ev.status,
                teams: layout.team_count(),
                max_teams: ev.schedule.max_teams,
            },
        ));
    }
    Ok(out)
}

/// The user's own partner invitations still open: events not cancelled or closed, with the
/// invitation link to resend.
pub(crate) async fn waiting_partners_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<Vec<WaitingItem>, StoreError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.join_token, c.name, s.partner_token FROM event_signups s \
         JOIN competitions c ON c.id = s.event_id \
         WHERE s.user1_id = ? AND s.partner_token IS NOT NULL AND s.complete_at IS NULL \
         AND c.status = 'setup' AND c.cancelled_at IS NULL AND c.signups_closed_at IS NULL \
         AND c.join_token IS NOT NULL AND (c.starts_at IS NULL OR c.starts_at > ?) \
         ORDER BY s.user1_at, s.id",
    )
    .bind(user)
    .bind(now)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(join_token, event_name, partner_token)| WaitingItem::Partner {
                join_token,
                event_name,
                partner_token,
            },
        )
        .collect())
}

/// Public events a user could still sign up for: sign-ups public and not closed, in setup, not
/// cancelled, the deadline and the start ahead, and the user neither organiser nor signed up.
/// `teams` counts the hand-added and the complete sign-ups, past the maximum too.
pub(crate) async fn open_events_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<Vec<OpenItem>, StoreError> {
    let rows = sqlx::query(
        "SELECT c.join_token, c.name, c.kind, c.starts_at, c.place, c.max_teams, \
           (SELECT COUNT(*) FROM competition_teams t WHERE t.event_id = c.id AND NOT EXISTS \
              (SELECT 1 FROM event_signups x WHERE x.taken_team_id = t.id)) \
           + (SELECT COUNT(*) FROM event_signups s \
              WHERE s.event_id = c.id AND s.complete_at IS NOT NULL) AS teams \
         FROM competitions c \
         WHERE c.signups = 'public' AND c.status = 'setup' AND c.cancelled_at IS NULL \
         AND c.signups_closed_at IS NULL AND c.join_token IS NOT NULL \
         AND c.deadline_at > ?1 AND c.starts_at > ?1 AND c.owner_user_id != ?2 \
         AND NOT EXISTS (SELECT 1 FROM event_signups s \
           WHERE s.event_id = c.id AND (s.user1_id = ?2 OR s.user2_id = ?2)) \
         ORDER BY c.starts_at, c.id LIMIT ?3",
    )
    .bind(now)
    .bind(user)
    .bind(i64::try_from(MAX_OPEN).unwrap_or(i64::MAX))
    .fetch_all(conn)
    .await?;
    let cap = u32::try_from(MAX_TEAMS).unwrap_or(u32::MAX);
    rows.iter()
        .map(|r| {
            let teams: i64 = r.try_get("teams")?;
            Ok(OpenItem::Event {
                join_token: r.try_get("join_token")?,
                name: r.try_get("name")?,
                event_kind: Kind::parse(r.try_get::<&str, _>("kind")?)?,
                starts_at: r.try_get("starts_at")?,
                place: r.try_get("place")?,
                teams: usize::try_from(teams).unwrap_or(0),
                max_teams: r.try_get::<Option<u32>, _>("max_teams")?.unwrap_or(cap),
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(StoreError::from)
}

/// `event_soon` for every dated event in setup (not cancelled, not started early) that starts
/// within the next day: to each account in the event (a team or taken entry; not the line, a
/// solo or a pending invitation), once per start time. The per-user dedupe key carries the
/// start, so a sign-up inside the last day is still told and a rescheduled event reminds again;
/// nothing else gates it. Returns how many notices were queued.
pub(crate) async fn queue_events_soon_in(tx: &mut Tx<'_>, now: i64) -> Result<usize, StoreError> {
    let due: Vec<(String, String, String, i64, Option<u32>)> = sqlx::query_as(
        "SELECT id, name, join_token, starts_at, max_teams FROM competitions \
         WHERE starts_at > ?1 AND starts_at <= ?1 + ?2 AND cancelled_at IS NULL \
         AND status = 'setup' AND join_token IS NOT NULL ORDER BY starts_at, id",
    )
    .bind(now)
    .bind(crate::calendar::DAY_SECS)
    .fetch_all(&mut **tx)
    .await?;
    let mut queued = 0;
    for (id, name, token, starts_at, max_teams) in due {
        let layout = layout_in(tx, &id, max_teams).await?;
        let mut users: Vec<&str> = layout.teams.iter().flat_map(Entry::users).collect();
        users.sort_unstable();
        users.dedup();
        let notices: Vec<Notice> = users
            .into_iter()
            .map(|user| Notice {
                dedupe: Some(format!("event-soon:{id}:{starts_at}:{user}")),
                user_id: user.to_owned(),
                payload: Payload {
                    kind: NoticeKind::EventSoon,
                    url: format!("/j/{token}"),
                    title: Some(name.clone()),
                    start: Some(starts_at),
                    ..Payload::default()
                },
            })
            .collect();
        queued += queue_in(tx, &notices, now).await?;
    }
    Ok(queued)
}
