//! The tournament's SQL: `bracket_matches` rows, loading a [`Bracket`], the draw, carrying out
//! a [`Plan`], withdrawing a match and putting waiting matches on free courts. The rules live in
//! `bracket.rs`; this file only moves them to and from SQLite. Like `courts.rs`, the `*_in`
//! functions run in the caller's transaction and take `now` as a parameter.

use sqlx::{Row, SqliteConnection};

use super::bracket::{Bracket, Layout, Node, Plan, Pos, Side, shuffle};
use super::settings_of;
use crate::competitions::courts::{
    advance_court_in, assign_in, courts_in, create_event_match_in, next_queued_in,
};
use crate::competitions::{Competition, teams_in};
use crate::store::{StoreError, Tx};

fn to_u32(n: i64) -> Result<u32, StoreError> {
    u32::try_from(n).map_err(|e| StoreError::Db(sqlx::Error::Decode(Box::new(e))))
}

/// The event's bracket, or `None` when it has no `bracket_matches` rows. One query: the rows
/// left-joined to `match_results` (the winner, as a [`Side`]) with an `EXISTS` for a match that
/// has started (an effective point event, or a result).
pub async fn load_in(
    conn: &mut SqliteConnection,
    ev: &Competition,
) -> Result<Option<Bracket>, StoreError> {
    let rows = sqlx::query(
        "SELECT b.place_from, b.size, b.idx, b.team_a_id, b.team_b_id, b.match_id, b.held, \
         r.winner, (r.match_id IS NOT NULL OR EXISTS (SELECT 1 FROM events e \
         WHERE e.match_id = b.match_id AND e.state = 'effective')) AS started \
         FROM bracket_matches b LEFT JOIN match_results r ON r.match_id = b.match_id \
         WHERE b.event_id = ?",
    )
    .bind(&ev.id)
    .fetch_all(conn)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut stored = Vec::with_capacity(rows.len());
    let mut teams = 0;
    for row in &rows {
        let pos = Pos {
            place_from: to_u32(row.try_get("place_from")?)?,
            size: to_u32(row.try_get("size")?)?,
            idx: to_u32(row.try_get("idx")?)?,
        };
        teams = teams.max(pos.size);
        let winner: Option<String> = row.try_get("winner")?;
        let node = Node {
            pos,
            teams: [row.try_get("team_a_id")?, row.try_get("team_b_id")?],
            match_id: row.try_get("match_id")?,
            started: row.try_get("started")?,
            winner: match winner.as_deref() {
                Some("a") => Some(Side::A),
                Some("b") => Some(Side::B),
                _ => None,
            },
            held: row.try_get("held")?,
        };
        stored.push(node);
    }
    let layout = Layout::new(teams as usize, settings_of(ev).full_placement)
        .ok_or_else(|| StoreError::Corrupt("the bracket has an unsupported size".into()))?;
    let mut nodes = Vec::with_capacity(stored.len());
    for pos in layout.positions() {
        let node = stored
            .iter()
            .find(|n| n.pos == pos)
            .ok_or_else(|| StoreError::Corrupt("the bracket is missing a match".into()))?;
        nodes.push(node.clone());
    }
    Ok(Some(Bracket { layout, nodes }))
}

/// A uniform random `u32` from the OS. Panicking on a missing OS randomness source is
/// deliberate, as for tokens: a draw must not be predictable.
#[allow(clippy::expect_used)]
fn random_u32() -> u32 {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    u32::from_le_bytes(bytes)
}

async fn set_teams_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    pos: Pos,
    side: Side,
    team: Option<&str>,
) -> Result<(), StoreError> {
    let sql = match side {
        Side::A => {
            "UPDATE bracket_matches SET team_a_id = ? WHERE event_id = ? AND place_from = ? \
             AND size = ? AND idx = ?"
        }
        Side::B => {
            "UPDATE bracket_matches SET team_b_id = ? WHERE event_id = ? AND place_from = ? \
             AND size = ? AND idx = ?"
        }
    };
    sqlx::query(sql)
        .bind(team)
        .bind(&ev.id)
        .bind(pos.place_from)
        .bind(pos.size)
        .bind(pos.idx)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn set_match_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    pos: Pos,
    match_id: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE bracket_matches SET match_id = ? WHERE event_id = ? AND place_from = ? \
         AND size = ? AND idx = ?",
    )
    .bind(match_id)
    .bind(&ev.id)
    .bind(pos.place_from)
    .bind(pos.size)
    .bind(pos.idx)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Make the event match of the node at `pos`, whose two teams are already stored, and link it.
async fn create_match_for_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    layout: &Layout,
    pos: Pos,
    now: i64,
) -> Result<(), StoreError> {
    let (a, b): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT team_a_id, team_b_id FROM bracket_matches WHERE event_id = ? \
         AND place_from = ? AND size = ? AND idx = ?",
    )
    .bind(&ev.id)
    .bind(pos.place_from)
    .bind(pos.size)
    .bind(pos.idx)
    .fetch_one(&mut **tx)
    .await?;
    let (Some(a), Some(b)) = (a, b) else {
        return Err(StoreError::Corrupt(
            "a match was created before both its teams were known".into(),
        ));
    };
    let made = create_event_match_in(tx, ev, &a, &b, Some(layout.label(pos)), None, now).await?;
    set_match_in(tx, ev, pos, &made.match_id).await
}

/// The draw, at Start: a row for every match of the layout, the teams shuffled into the first
/// round, its matches created in `idx` order, then the free courts filled.
pub async fn draw_in(tx: &mut Tx<'_>, ev: &Competition, now: i64) -> Result<(), StoreError> {
    let teams = teams_in(tx, &ev.id).await?;
    let layout = Layout::new(teams.len(), settings_of(ev).full_placement)
        .ok_or(StoreError::Conflict("team_count"))?;
    let positions = layout.positions();
    for pos in &positions {
        sqlx::query(
            "INSERT INTO bracket_matches (event_id, place_from, size, idx) VALUES (?, ?, ?, ?)",
        )
        .bind(&ev.id)
        .bind(pos.place_from)
        .bind(pos.size)
        .bind(pos.idx)
        .execute(&mut **tx)
        .await?;
    }
    let mut ids: Vec<String> = teams.into_iter().map(|t| t.id).collect();
    shuffle(&mut ids, random_u32);
    // The first round is the positions that have no source, in `idx` order.
    let first_round: Vec<Pos> = positions
        .into_iter()
        .filter(|p| layout.source(*p, Side::A).is_none())
        .collect();
    for (pos, pair) in first_round.iter().zip(ids.chunks(2)) {
        if let [a, b] = pair {
            set_teams_in(tx, ev, *pos, Side::A, Some(a)).await?;
            set_teams_in(tx, ev, *pos, Side::B, Some(b)).await?;
        }
    }
    for pos in &first_round {
        create_match_for_in(tx, ev, &layout, *pos, now).await?;
    }
    fill_courts_in(tx, ev).await
}

/// Carry out a plan, in this order: withdraw the matches, set the slots, create the matches
/// (labelled by the layout), write the holds, then fill the free courts if the event is
/// running. Returns the withdrawn match ids, so the caller can close their live streams.
pub async fn apply_plan_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    bracket: &Bracket,
    plan: &Plan,
    now: i64,
) -> Result<Vec<String>, StoreError> {
    for match_id in &plan.withdraw {
        withdraw_in(tx, ev, match_id).await?;
    }
    for (pos, side, team) in &plan.set {
        set_teams_in(tx, ev, *pos, *side, team.as_deref()).await?;
    }
    for pos in &plan.create {
        create_match_for_in(tx, ev, &bracket.layout, *pos, now).await?;
    }
    for (pos, held) in &plan.held {
        sqlx::query(
            "UPDATE bracket_matches SET held = ? WHERE event_id = ? AND place_from = ? \
             AND size = ? AND idx = ?",
        )
        .bind(i64::from(*held))
        .bind(&ev.id)
        .bind(pos.place_from)
        .bind(pos.size)
        .bind(pos.idx)
        .execute(&mut **tx)
        .await?;
    }
    if ev.status == crate::competitions::Status::Running {
        fill_courts_in(tx, ev).await?;
    }
    Ok(plan.withdraw.clone())
}

/// Take a match out of the tournament: off its court the phase 6 way (an unfinished match is
/// unassigned; the finished current match advances its court), then delete its `matches` row.
/// The cascade removes its log, players, result and event link, and the bracket row's
/// `match_id` goes back to `NULL`.
pub async fn withdraw_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
) -> Result<(), StoreError> {
    let finished: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
            .bind(match_id)
            .fetch_optional(&mut **tx)
            .await?;
    if finished.is_none() {
        assign_in(tx, ev, match_id, None).await?;
    } else {
        let current_on: Option<String> =
            sqlx::query_scalar("SELECT id FROM courts WHERE current_match_id = ?")
                .bind(match_id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(court_id) = current_on {
            advance_court_in(tx, ev, &court_id).await?;
        }
    }
    sqlx::query("DELETE FROM matches WHERE id = ?")
        .bind(match_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Put waiting bracket matches (created, unfinished, on no court) onto free courts, one per
/// court, in `ordinal` order. A court is free when its current match is finished or absent and
/// nothing is queued on it; courts are taken in `position` order. A match already on a court,
/// by hand or by an earlier fill, is never moved.
pub async fn fill_courts_in(tx: &mut Tx<'_>, ev: &Competition) -> Result<(), StoreError> {
    let mut waiting: std::collections::VecDeque<String> = sqlx::query_scalar(
        "SELECT cm.match_id FROM competition_matches cm \
         JOIN bracket_matches b ON b.match_id = cm.match_id \
         WHERE cm.event_id = ? AND cm.court_id IS NULL \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = cm.match_id) \
         ORDER BY cm.ordinal",
    )
    .bind(&ev.id)
    .fetch_all(&mut **tx)
    .await?
    .into();
    for court in courts_in(tx, &ev.id).await? {
        if waiting.is_empty() {
            break;
        }
        if let Some(current) = &court.current_match_id {
            let finished: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
                    .bind(current)
                    .fetch_optional(&mut **tx)
                    .await?;
            if finished.is_none() {
                continue;
            }
        }
        if next_queued_in(tx, &court.id).await?.is_some() {
            continue;
        }
        if let Some(match_id) = waiting.pop_front() {
            assign_in(tx, ev, &match_id, Some(&court.id)).await?;
        }
    }
    Ok(())
}
