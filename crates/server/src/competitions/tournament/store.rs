//! The tournament's SQL: `bracket_matches` rows, loading a [`Bracket`], the draw, carrying out
//! a [`Plan`], withdrawing a match and putting waiting matches on free courts. The rules live in
//! `bracket.rs`; this file only moves them to and from SQLite. Like `courts.rs`, the `*_in`
//! functions run in the caller's transaction and take `now` as a parameter.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use super::bracket::{Bracket, Layout, Node, Plan, Pos, Side, Source, shuffle};
use super::settings_of;
use crate::competitions::courts::{
    advance_court_in, assign_in, courts_in, create_event_match_in, next_queued_in,
};
use crate::competitions::places::{Place, validate_places, write_placings_in};
use crate::competitions::{Audience, Competition, Kind, Status, competition_in, teams_in};
use crate::store::{Store, StoreError, Tx};

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

/// The draw, at Start: a row for every match of the layout, then [`seed_in`].
pub async fn draw_in(tx: &mut Tx<'_>, ev: &Competition, now: i64) -> Result<(), StoreError> {
    let teams = teams_in(tx, &ev.id).await?;
    let layout = Layout::new(teams.len(), settings_of(ev).full_placement)
        .ok_or(StoreError::Conflict("team_count"))?;
    for pos in &layout.positions() {
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
    seed_in(tx, ev, &layout, now).await
}

/// Fill empty bracket rows: the teams shuffled into the first round, its matches created in
/// `idx` order, then the free courts filled.
async fn seed_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    layout: &Layout,
    now: i64,
) -> Result<(), StoreError> {
    let positions = layout.positions();
    let mut ids: Vec<String> = teams_in(tx, &ev.id)
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect();
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
        create_match_for_in(tx, ev, layout, *pos, now).await?;
    }
    fill_courts_in(tx, ev).await
}

/// Draw again: needs a running event (`Conflict("event_not_running")`) whose bracket matches
/// have not started (`Conflict("play_started")`). Every bracket match is withdrawn, every slot
/// and hold cleared, and the teams shuffled into the first round again. Returns the withdrawn
/// match ids.
pub async fn redraw_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    now: i64,
) -> Result<Vec<String>, StoreError> {
    if ev.status != Status::Running {
        return Err(StoreError::Conflict("event_not_running"));
    }
    let bracket = load_in(tx, ev).await?.ok_or(StoreError::NotFound)?;
    if bracket.nodes.iter().any(|n| n.started) {
        return Err(StoreError::Conflict("play_started"));
    }
    let withdrawn: Vec<String> = bracket
        .nodes
        .iter()
        .filter_map(|n| n.match_id.clone())
        .collect();
    for match_id in &withdrawn {
        withdraw_in(tx, ev, match_id).await?;
    }
    sqlx::query(
        "UPDATE bracket_matches SET team_a_id = NULL, team_b_id = NULL, match_id = NULL, \
         held = 0 WHERE event_id = ?",
    )
    .bind(&ev.id)
    .execute(&mut **tx)
    .await?;
    seed_in(tx, ev, &bracket.layout, now).await?;
    Ok(withdrawn)
}

/// The coordinator's answer to a held match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    /// Discard the later matches the old winner played and make them again.
    Replay,
    /// Keep the later matches as played.
    Keep,
}

/// Decide on the held bracket match `match_id`: unknown is `NotFound`, not held is
/// `Conflict("no_decision")`. `Keep` clears the hold (after Finish it also writes the placings
/// again, with the event's `finished_at`). `Replay` needs a running event
/// (`Conflict("event_finished")`) and carries out `Bracket::replay`; then every hold still set
/// is looked at again, as a result would be, since the matches it was waiting on may be gone.
/// A team kept as played stays in its slot even if a later `Replay` elsewhere discards the match
/// it played on. Returns the withdrawn match ids.
pub async fn decide_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
    choice: Choice,
    now: i64,
) -> Result<Vec<String>, StoreError> {
    let bracket = load_in(tx, ev).await?.ok_or(StoreError::NotFound)?;
    let node = bracket
        .node_by_match(match_id)
        .ok_or(StoreError::NotFound)?;
    if !node.held {
        return Err(StoreError::Conflict("no_decision"));
    }
    let pos = node.pos;
    match choice {
        Choice::Keep => {
            let plan = Plan {
                held: vec![(pos, false)],
                ..Plan::default()
            };
            apply_plan_in(tx, ev, &bracket, &plan, now).await?;
            if ev.status == Status::Finished {
                rewrite_placings_in(tx, ev, now).await?;
            }
            Ok(Vec::new())
        }
        Choice::Replay => {
            if ev.status != Status::Running {
                return Err(StoreError::Conflict("event_finished"));
            }
            let plan = bracket.replay(pos);
            let mut withdrawn = apply_plan_in(tx, ev, &bracket, &plan, now).await?;
            for p in bracket.layout.positions() {
                let Some(bracket) = load_in(tx, ev).await? else {
                    break;
                };
                if !bracket.node(p).is_some_and(|n| n.held) {
                    continue;
                }
                let plan = bracket.on_result(p);
                withdrawn.extend(apply_plan_in(tx, ev, &bracket, &plan, now).await?);
            }
            Ok(withdrawn)
        }
    }
}

/// The bracket's places, if it is complete and they pass `validate_places`.
async fn valid_places(
    conn: &mut SqliteConnection,
    ev: &Competition,
    bracket: &Bracket,
) -> Result<Option<Vec<Place>>, StoreError> {
    let Some(places) = bracket.places() else {
        return Ok(None);
    };
    let ids: Vec<String> = teams_in(conn, &ev.id)
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect();
    Ok(validate_places(&ids, &places).is_ok().then_some(places))
}

/// After Finish: write the placings again from the bracket, keeping the event's
/// `finished_at`, when it is complete and its places are valid; else leave them.
pub async fn rewrite_placings_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    now: i64,
) -> Result<(), StoreError> {
    let Some(bracket) = load_in(tx, ev).await? else {
        return Ok(());
    };
    if let Some(places) = valid_places(tx, ev, &bracket).await? {
        let teams = teams_in(tx, &ev.id).await?;
        write_placings_in(tx, ev, &teams, &places, ev.finished_at.unwrap_or(now)).await?;
    }
    Ok(())
}

/// One node of the bracket as `kind_data` shows it.
#[derive(Serialize)]
struct NodeOut<'a> {
    place_from: u32,
    size: u32,
    idx: u32,
    label: String,
    teams: &'a [Option<String>; 2],
    from: [Option<Source>; 2],
    match_id: Option<&'a str>,
    held: bool,
    /// Coordinator only, on a held node: the labels of the matches Replay would discard.
    #[serde(skip_serializing_if = "Option::is_none")]
    replay_discards: Option<Vec<String>>,
}

/// The bracket as `kind_data`.
#[derive(Serialize)]
struct BracketOut<'a> {
    full_placement: bool,
    teams: u32,
    nodes: Vec<NodeOut<'a>>,
    places: Option<Vec<Place>>,
    /// Coordinator only: the event runs and no bracket match has started.
    #[serde(skip_serializing_if = "Option::is_none")]
    can_redraw: Option<bool>,
}

/// The bracket as `kind_data` (see the plan's wire shapes): `None` with no bracket and for a
/// court. `replay_discards` (on held nodes) and `can_redraw` are for the coordinator only. No
/// token of any kind is in it.
pub async fn kind_json(
    conn: &mut SqliteConnection,
    ev: &Competition,
    audience: &Audience,
) -> Result<Option<serde_json::Value>, StoreError> {
    if matches!(audience, Audience::Court { .. }) {
        return Ok(None);
    }
    let Some(bracket) = load_in(conn, ev).await? else {
        return Ok(None);
    };
    let coordinator = *audience == Audience::Coordinator;
    let layout = &bracket.layout;
    let nodes = bracket
        .nodes
        .iter()
        .map(|n| NodeOut {
            place_from: n.pos.place_from,
            size: n.pos.size,
            idx: n.pos.idx,
            label: layout.label(n.pos),
            teams: &n.teams,
            from: [layout.source(n.pos, Side::A), layout.source(n.pos, Side::B)],
            match_id: n.match_id.as_deref(),
            held: n.held,
            replay_discards: (coordinator && n.held).then(|| {
                bracket
                    .replay_discards(n.pos)
                    .into_iter()
                    .map(|p| layout.label(p))
                    .collect()
            }),
        })
        .collect();
    let out = BracketOut {
        full_placement: layout.full_placement(),
        teams: layout.teams(),
        nodes,
        places: valid_places(conn, ev, &bracket).await?,
        can_redraw: coordinator
            .then(|| ev.status == Status::Running && !bracket.nodes.iter().any(|n| n.started)),
    };
    serde_json::to_value(out)
        .map(Some)
        .map_err(|e| StoreError::Corrupt(format!("cannot encode the bracket: {e}")))
}

/// The owner's tournament `id` in `tx`: another owner's, an unknown one or a non-tournament
/// event is `NotFound`.
async fn tournament_in(tx: &mut Tx<'_>, owner: &str, id: &str) -> Result<Competition, StoreError> {
    competition_in(tx, owner, id)
        .await?
        .filter(|ev| ev.kind == Kind::Tournament)
        .ok_or(StoreError::NotFound)
}

impl Store {
    /// [`redraw_in`] on the owner's tournament, in one transaction.
    pub async fn redraw_bracket(
        &self,
        owner: &str,
        id: &str,
        now: i64,
    ) -> Result<Vec<String>, StoreError> {
        let mut tx = self.begin().await?;
        let ev = tournament_in(&mut tx, owner, id).await?;
        let withdrawn = redraw_in(&mut tx, &ev, now).await?;
        tx.commit().await?;
        Ok(withdrawn)
    }

    /// [`decide_in`] on the owner's tournament, in one transaction.
    pub async fn decide_bracket(
        &self,
        owner: &str,
        id: &str,
        match_id: &str,
        choice: Choice,
        now: i64,
    ) -> Result<Vec<String>, StoreError> {
        let mut tx = self.begin().await?;
        let ev = tournament_in(&mut tx, owner, id).await?;
        let withdrawn = decide_in(&mut tx, &ev, match_id, choice, now).await?;
        tx.commit().await?;
        Ok(withdrawn)
    }

    /// Put waiting bracket matches on the free courts of the owner's running tournament
    /// (`Conflict("event_not_running")` otherwise), in one transaction.
    pub async fn fill_bracket_courts(&self, owner: &str, id: &str) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let ev = tournament_in(&mut tx, owner, id).await?;
        if ev.status != Status::Running {
            return Err(StoreError::Conflict("event_not_running"));
        }
        fill_courts_in(&mut tx, &ev).await?;
        tx.commit().await?;
        Ok(())
    }
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
    if ev.status == Status::Running {
        fill_courts_in(tx, ev).await?;
    }
    Ok(plan.withdraw.clone())
}

/// Take a match out of the tournament: off its court the phase 6 way (an unfinished match is
/// unassigned; the finished current match advances its court), then delete its `matches` row.
/// The cascade removes its log, players, result and event link, and the bracket row's
/// `match_id` goes back to `NULL`. A match that is not this event's is `NotFound` and nothing
/// is touched. Errors on a finished event (`assign_in` and `advance_court_in` refuse it).
pub async fn withdraw_in(
    tx: &mut Tx<'_>,
    ev: &Competition,
    match_id: &str,
) -> Result<(), StoreError> {
    let linked: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM competition_matches WHERE match_id = ? AND event_id = ? LIMIT 1",
    )
    .bind(match_id)
    .bind(&ev.id)
    .fetch_optional(&mut **tx)
    .await?;
    if linked.is_none() {
        return Err(StoreError::NotFound);
    }
    let finished: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
            .bind(match_id)
            .fetch_optional(&mut **tx)
            .await?;
    if finished.is_none() {
        assign_in(tx, ev, match_id, None).await?;
    } else {
        let current_on: Option<String> =
            sqlx::query_scalar("SELECT id FROM courts WHERE current_match_id = ? AND event_id = ?")
                .bind(match_id)
                .bind(&ev.id)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some(court_id) = current_on {
            advance_court_in(tx, ev, &court_id).await?;
        }
    }
    sqlx::query(
        "DELETE FROM matches WHERE id = ? \
         AND id IN (SELECT match_id FROM competition_matches WHERE event_id = ?)",
    )
    .bind(match_id)
    .bind(&ev.id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Put waiting bracket matches (created, unfinished, on no court) onto free courts, one per
/// court, in `ordinal` order. A court is free when its current match is finished or absent and
/// nothing is queued on it; courts are taken in `position` order. A match already on a court,
/// by hand or by an earlier fill, is never moved. The final (place 1, size 2) is the last match
/// onto a court: it is skipped while any other bracket position has no match yet or has an
/// unfinished match on no court, since a 5th/7th-place match can be made after the final.
pub async fn fill_courts_in(tx: &mut Tx<'_>, ev: &Competition) -> Result<(), StoreError> {
    let mut waiting: std::collections::VecDeque<String> = sqlx::query_scalar(
        "SELECT cm.match_id FROM competition_matches cm \
         JOIN bracket_matches b ON b.match_id = cm.match_id \
         WHERE cm.event_id = ? AND cm.court_id IS NULL \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = cm.match_id) \
         AND NOT (b.place_from = 1 AND b.size = 2 AND EXISTS ( \
           SELECT 1 FROM bracket_matches o LEFT JOIN competition_matches ocm \
             ON ocm.match_id = o.match_id \
           WHERE o.event_id = cm.event_id AND NOT (o.place_from = 1 AND o.size = 2) \
           AND (o.match_id IS NULL OR (ocm.court_id IS NULL AND NOT EXISTS ( \
             SELECT 1 FROM match_results orr WHERE orr.match_id = o.match_id))))) \
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
