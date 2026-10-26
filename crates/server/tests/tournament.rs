#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, FinalRequest, MatchView, apply_command,
    record_final,
};
use padel_score_server::competitions::courts::{Court, courts_in, event_matches_in};
use padel_score_server::competitions::tournament::bracket::{Plan, Pos, Side};
use padel_score_server::competitions::tournament::store::{
    Choice, apply_plan_in, fill_courts_in, load_in, withdraw_in,
};
use padel_score_server::competitions::view::court_state_by_token;
use padel_score_server::competitions::{Competition, Kind, NewCompetition, Status};
use padel_score_server::db;
use padel_score_server::store::{EventState, Store, StoreError};
use padel_scoring::{
    DecidingSet, DeuceRule, FinalScore, Format, SetRules, SetScore, SetsFormat, Team,
};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

const A: &str = "user-a";
const NOW: i64 = 1_000;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn player(s: &Store, name: &str) -> String {
    s.create_or_get_player(A, name, 1).await.unwrap().0.id
}

/// A tournament of A with `n` teams and `courts` courts, still in setup.
async fn event(
    s: &Store,
    settings: Option<Value>,
    n: usize,
    courts: usize,
) -> (Competition, Vec<String>) {
    let ev = s
        .create_competition(
            A,
            NewCompetition {
                name: "Cup".into(),
                kind: Kind::Tournament,
                format: one_set(),
                settings,
            },
            10,
        )
        .await
        .unwrap();
    let mut teams = Vec::new();
    for i in 0..n {
        let players = [
            player(s, &format!("Ana{i}")).await,
            player(s, &format!("Rui{i}")).await,
        ];
        teams.push(s.add_team(A, &ev.id, players, None, 10).await.unwrap().id);
    }
    for _ in 0..courts {
        s.add_court(A, &ev.id, None).await.unwrap();
    }
    (ev, teams)
}

fn full() -> Option<Value> {
    Some(json!({ "full_placement": true }))
}

async fn start(s: &Store, ev: &Competition) -> Result<Competition, StoreError> {
    s.start_competition(A, &ev.id, 20).await
}

/// The bracket rows: (place_from, size, idx, team_a, team_b, match_id), in the same order as
/// a bracket draws them.
type Row6 = (
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn rows(pool: &SqlitePool, ev: &Competition) -> Vec<Row6> {
    sqlx::query(
        "SELECT place_from, size, idx, team_a_id, team_b_id, match_id FROM bracket_matches \
         WHERE event_id = ? ORDER BY size DESC, place_from, idx",
    )
    .bind(&ev.id)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
    .collect()
}

/// The match id of the quarter-final (or first-round match) `n` (1-based) of an 8-team draw.
async fn qf(pool: &SqlitePool, ev: &Competition, n: i64) -> String {
    rows(pool, ev)
        .await
        .into_iter()
        .find(|r| r.0 == 1 && r.1 == 8 && r.2 == n - 1)
        .unwrap()
        .5
        .unwrap()
}

async fn court_of(pool: &SqlitePool, match_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT court_id FROM competition_matches WHERE match_id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn courts(pool: &SqlitePool, ev: &Competition) -> Vec<Court> {
    let mut conn = pool.acquire().await.unwrap();
    courts_in(&mut conn, &ev.id).await.unwrap()
}

/// Finish a match through its scorer token: 24 points to side A in one request (6-0 in a
/// one-set match), as a court phone would.
async fn finish_by_scorer(s: &Store, match_id: &str) {
    let (m, _) = s.snapshot(match_id).await.unwrap().unwrap();
    let req = CommandRequest {
        client_id: format!("finish-{match_id}"),
        batch: false,
        base_revision: 0,
        commands: vec![Command::Point { team: Team::A }; 24],
    };
    applied(
        apply_command(s, &m, req, NOW, Authority::Scorer)
            .await
            .unwrap(),
    );
}

fn applied(o: CommandOutcome) -> MatchView {
    match o {
        CommandOutcome::Applied(v) => v,
        other => panic!("expected Applied, got {other:?}"),
    }
}

/// The coordinator types `[a-b]` for a match (a correction when it already has a result).
async fn typed(s: &Store, match_id: &str, a: u8, b: u8) -> CommandOutcome {
    let (m, _) = s.snapshot(match_id).await.unwrap().unwrap();
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
    let req = FinalRequest {
        client_id: format!("typed-{n}"),
        base_revision: None,
        score: FinalScore::Sets(vec![SetScore {
            a,
            b,
            tiebreak: None,
        }]),
    };
    record_final(s, &m, req, Authority::Owner, NOW)
        .await
        .unwrap()
}

static CLIENT: AtomicU64 = AtomicU64::new(0);

/// The coordinator types `[6-0]` (winner A) or `[0-6]` (winner B).
async fn finish(s: &Store, match_id: &str, winner: Team) {
    let (a, b) = match winner {
        Team::A => (6, 0),
        Team::B => (0, 6),
    };
    applied(typed(s, match_id, a, b).await);
}

/// The match's current scorer token (it changes when the match leaves a court).
async fn token(pool: &SqlitePool, match_id: &str) -> String {
    sqlx::query_scalar("SELECT scorer_token FROM matches WHERE id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// One live command through the scorer path: the match is found by its scorer token.
async fn scorer_command(s: &Store, scorer_token: &str, command: Command) -> CommandOutcome {
    let m = s.by_scorer_token(scorer_token).await.unwrap().unwrap();
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
    let req = CommandRequest {
        client_id: format!("scorer-{n}"),
        batch: false,
        base_revision: m.revision,
        commands: vec![command],
    };
    apply_command(s, &m, req, NOW, Authority::Scorer)
        .await
        .unwrap()
}

/// A point to side A through the scorer path.
async fn score_point(s: &Store, scorer_token: &str) -> CommandOutcome {
    scorer_command(s, scorer_token, Command::Point { team: Team::A }).await
}

/// Undo the latest effective event through the scorer path.
async fn undo(s: &Store, scorer_token: &str) -> CommandOutcome {
    let m = s.by_scorer_token(scorer_token).await.unwrap().unwrap();
    let events = s.events(&m.id).await.unwrap();
    let target_seq = events
        .iter()
        .filter(|e| e.state == EventState::Effective)
        .map(|e| e.seq)
        .max()
        .unwrap();
    scorer_command(s, scorer_token, Command::Undo { target_seq }).await
}

async fn fill(s: &Store, ev: &Competition) {
    let mut tx = s.begin().await.unwrap();
    fill_courts_in(&mut tx, ev).await.unwrap();
    tx.commit().await.unwrap();
}

async fn assignments(
    pool: &SqlitePool,
    ev: &Competition,
) -> Vec<(String, Option<String>, Option<i64>)> {
    let mut conn = pool.acquire().await.unwrap();
    event_matches_in(&mut conn, &ev.id)
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.match_id, m.court_id, m.court_seq))
        .collect()
}

#[tokio::test]
async fn settings_are_checked() {
    let (s, _) = setup().await;
    let (ev, _) = event(&s, None, 0, 0).await;
    assert_eq!(ev.settings, json!({ "full_placement": false }));
    let (ev, _) = event(&s, Some(json!({})), 0, 0).await;
    assert_eq!(ev.settings, json!({ "full_placement": false }));
    let (ev, _) = event(&s, full(), 0, 0).await;
    assert_eq!(ev.settings, json!({ "full_placement": true }));
    for bad in [
        json!({ "full_placement": "yes" }),
        json!({ "x": 1 }),
        json!({ "full_placement": true, "x": 1 }),
        json!(null),
        json!([]),
    ] {
        let err = s
            .create_competition(
                A,
                NewCompetition {
                    name: "Cup".into(),
                    kind: Kind::Tournament,
                    format: one_set(),
                    settings: Some(bad.clone()),
                },
                10,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("settings")),
            "{bad}: {err:?}"
        );
    }
}

#[tokio::test]
async fn start_needs_power_of_two() {
    let (s, pool) = setup().await;
    for n in [3, 6, 12] {
        let (ev, _) = event(&s, None, n, 1).await;
        let err = start(&s, &ev).await.unwrap_err();
        assert!(
            matches!(err, StoreError::Conflict("team_count")),
            "{n}: {err:?}"
        );
        let got = s.competition(A, &ev.id).await.unwrap().unwrap();
        assert_eq!(got.status, Status::Setup);
        assert!(rows(&pool, &ev).await.is_empty());
    }
    for n in [4, 8] {
        let (ev, _) = event(&s, None, n, 1).await;
        let started = start(&s, &ev).await.unwrap();
        assert_eq!(started.status, Status::Running, "{n}");
    }
}

#[tokio::test]
async fn start_draws_and_fills_courts() {
    let (s, pool) = setup().await;
    let (ev, teams) = event(&s, full(), 8, 2).await;
    start(&s, &ev).await.unwrap();
    let all = rows(&pool, &ev).await;
    assert_eq!(all.len(), 12);
    let mut conn = pool.acquire().await.unwrap();
    let made = event_matches_in(&mut conn, &ev.id).await.unwrap();
    drop(conn);
    let labels: Vec<_> = made.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Quarter-final 1",
            "Quarter-final 2",
            "Quarter-final 3",
            "Quarter-final 4"
        ]
    );
    let mut seen = HashSet::new();
    for m in &made {
        assert!(seen.insert(m.team_a_id.clone()));
        assert!(seen.insert(m.team_b_id.clone()));
    }
    assert_eq!(seen, teams.iter().cloned().collect());
    // Every first-round row names the teams of its match.
    for (i, m) in made.iter().enumerate() {
        let r = &all[i];
        assert_eq!((r.1, r.2), (8, i64::try_from(i).unwrap()));
        assert_eq!(r.3.as_deref(), Some(m.team_a_id.as_str()));
        assert_eq!(r.4.as_deref(), Some(m.team_b_id.as_str()));
        assert_eq!(r.5.as_deref(), Some(m.match_id.as_str()));
    }
    // Later rows have no match and no teams yet.
    assert!(
        all[4..]
            .iter()
            .all(|r| r.3.is_none() && r.4.is_none() && r.5.is_none())
    );
    let cs = courts(&pool, &ev).await;
    assert_eq!(
        cs[0].current_match_id.as_deref(),
        Some(made[0].match_id.as_str())
    );
    assert_eq!(
        cs[1].current_match_id.as_deref(),
        Some(made[1].match_id.as_str())
    );
    assert_eq!(made[2].court_id, None);
    assert_eq!(made[3].court_id, None);
    assert_eq!(made[0].court_id.as_deref(), Some(cs[0].id.as_str()));
    assert_eq!(made[1].court_id.as_deref(), Some(cs[1].id.as_str()));
}

#[tokio::test]
async fn draw_is_random() {
    let (s, pool) = setup().await;
    let mut orders = HashSet::new();
    for _ in 0..20 {
        let (ev, teams) = event(&s, None, 8, 1).await;
        start(&s, &ev).await.unwrap();
        let order: Vec<_> = rows(&pool, &ev)
            .await
            .into_iter()
            .filter(|r| r.1 == 8)
            .flat_map(|r| [r.3.unwrap(), r.4.unwrap()])
            .collect();
        // Team ids differ between events, so compare by index in the event's team list.
        let ids: HashMap<_, _> = teams.iter().enumerate().map(|(i, t)| (t, i)).collect();
        orders.insert(order.iter().map(|t| ids[&t]).collect::<Vec<_>>());
    }
    assert!(orders.len() > 1, "twenty draws were all the same");
}

#[tokio::test]
async fn fill_never_double_books() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q3) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 3).await);
    let cs = courts(&pool, &ev).await;
    finish_by_scorer(&s, &q1).await;
    fill(&s, &ev).await;
    assert_eq!(
        court_of(&pool, &q3).await.as_deref(),
        Some(cs[0].id.as_str())
    );
    // QF1 is still the court's finished current match; QF3 waits behind it.
    let cs_after = courts(&pool, &ev).await;
    assert_eq!(cs_after[0].current_match_id.as_deref(), Some(q1.as_str()));
    let q4 = qf(&pool, &ev, 4).await;
    assert_eq!(court_of(&pool, &q4).await, None, "court 2 is busy");
    let before = assignments(&pool, &ev).await;
    fill(&s, &ev).await;
    assert_eq!(assignments(&pool, &ev).await, before);
}

#[tokio::test]
async fn fill_leaves_hand_assigned_matches() {
    let (s, pool) = setup().await;
    let (ev, teams) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let cs = courts(&pool, &ev).await;
    let (q1, q3, q4) = (
        qf(&pool, &ev, 1).await,
        qf(&pool, &ev, 3).await,
        qf(&pool, &ev, 4).await,
    );
    s.assign_match(A, &ev.id, &q4, Some(&cs[1].id))
        .await
        .unwrap();
    // A match the coordinator made by hand is outside the bracket: the fill never places it,
    // even on a court that stands free.
    let made = s
        .create_event_match(A, &ev.id, &teams[0], &teams[1], None, None, NOW)
        .await
        .unwrap();
    let c3 = s.add_court(A, &ev.id, None).await.unwrap();
    finish_by_scorer(&s, &q1).await;
    fill(&s, &ev).await;
    assert_eq!(court_of(&pool, &made.match_id).await, None);
    let c3_now = courts(&pool, &ev)
        .await
        .into_iter()
        .find(|c| c.id == c3.id)
        .unwrap();
    assert_eq!(c3_now.current_match_id, None);
    assert_eq!(
        court_of(&pool, &q4).await.as_deref(),
        Some(cs[1].id.as_str())
    );
    assert_eq!(
        court_of(&pool, &q3).await.as_deref(),
        Some(cs[0].id.as_str())
    );
}

#[tokio::test]
async fn off_mode_has_eight_matches() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, None, 8, 1).await;
    let ev = start(&s, &ev).await.unwrap();
    assert_eq!(rows(&pool, &ev).await.len(), 8);
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(event_matches_in(&mut conn, &ev.id).await.unwrap().len(), 4);
}

const fn pos(place_from: u32, size: u32, idx: u32) -> Pos {
    Pos {
        place_from,
        size,
        idx,
    }
}

const QF1: Pos = pos(1, 8, 0);
const QF2: Pos = pos(1, 8, 1);
const QF3: Pos = pos(1, 8, 2);
const QF4: Pos = pos(1, 8, 3);
const SF1: Pos = pos(1, 4, 0);
const PS1: Pos = pos(5, 4, 0);
const FINAL: Pos = pos(1, 2, 0);

/// When the coordinator presses Finish in these tests.
const FINISHED_AT: i64 = 5_000;

async fn node(pool: &SqlitePool, ev: &Competition, p: Pos) -> Row6 {
    rows(pool, ev)
        .await
        .into_iter()
        .find(|r| (r.0, r.1, r.2) == (i64::from(p.place_from), i64::from(p.size), i64::from(p.idx)))
        .unwrap()
}

/// The team on `side` of a bracket row.
fn side(r: &Row6, t: Team) -> String {
    match t {
        Team::A => r.3.clone().unwrap(),
        Team::B => r.4.clone().unwrap(),
    }
}

async fn is_held(pool: &SqlitePool, ev: &Competition, p: Pos) -> bool {
    sqlx::query_scalar(
        "SELECT held FROM bracket_matches WHERE event_id = ? AND place_from = ? AND size = ? \
         AND idx = ?",
    )
    .bind(&ev.id)
    .bind(p.place_from)
    .bind(p.size)
    .bind(p.idx)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn exists(pool: &SqlitePool, match_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM matches WHERE id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
        == 1
}

async fn label_of(pool: &SqlitePool, match_id: &str) -> String {
    sqlx::query_scalar("SELECT label FROM competition_matches WHERE match_id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn effective_events(s: &Store, match_id: &str) -> usize {
    s.events(match_id)
        .await
        .unwrap()
        .iter()
        .filter(|e| e.state == EventState::Effective)
        .count()
}

/// The invariant: a bracket row with a match names the same two teams as the match's link.
async fn check_rows(pool: &SqlitePool, ev: &Competition) {
    let bad: Vec<(Option<String>, Option<String>, String, String)> = sqlx::query_as(
        "SELECT b.team_a_id, b.team_b_id, cm.team_a_id, cm.team_b_id FROM bracket_matches b \
         JOIN competition_matches cm ON cm.match_id = b.match_id WHERE b.event_id = ? \
         AND (b.team_a_id IS NOT cm.team_a_id OR b.team_b_id IS NOT cm.team_b_id)",
    )
    .bind(&ev.id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(bad.is_empty(), "rows differ from their matches: {bad:?}");
    let dangling: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bracket_matches b WHERE b.event_id = ? AND b.match_id IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM competition_matches cm WHERE cm.match_id = b.match_id)",
    )
    .bind(&ev.id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(dangling, 0);
}

/// The unfinished bracket matches, in creation order.
async fn unfinished(pool: &SqlitePool, ev: &Competition) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT cm.match_id FROM competition_matches cm \
         JOIN bracket_matches b ON b.match_id = cm.match_id WHERE cm.event_id = ? \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = cm.match_id) \
         ORDER BY cm.ordinal",
    )
    .bind(&ev.id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Play the whole bracket, side A winning every match, oldest match first; with `check`, the
/// rows are checked against their matches after every result.
async fn play_all(s: &Store, pool: &SqlitePool, ev: &Competition, check: bool) {
    while let Some(m) = unfinished(pool, ev).await.into_iter().next() {
        finish(s, &m, Team::A).await;
        if check {
            check_rows(pool, ev).await;
        }
    }
}

/// The written placings: (player, place, place_to, field_size, finished_at), sorted.
async fn placing_rows(pool: &SqlitePool, ev: &Competition) -> Vec<(String, i64, i64, i64, i64)> {
    sqlx::query_as(
        "SELECT player_id, place, place_to, field_size, finished_at FROM placings \
         WHERE event_id = ? ORDER BY player_id",
    )
    .bind(&ev.id)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Each team's written place: team id -> (place, place_to).
async fn team_places(pool: &SqlitePool, ev: &Competition) -> HashMap<String, (u32, u32)> {
    let mut conn = pool.acquire().await.unwrap();
    padel_score_server::competitions::places::places(&mut conn, &ev.id)
        .await
        .unwrap()
        .into_iter()
        .map(|p| (p.team_id, (p.place, p.place_to)))
        .collect()
}

async fn finish_event(s: &Store, ev: &Competition) -> Result<Competition, StoreError> {
    s.finish_competition(A, &ev.id, None, FINISHED_AT)
        .await
        .map(|(ev, _)| ev)
}

async fn withdraw(s: &Store, ev: &Competition, match_id: &str) -> Result<(), StoreError> {
    let mut tx = s.begin().await.unwrap();
    withdraw_in(&mut tx, ev, match_id).await?;
    tx.commit().await.unwrap();
    Ok(())
}

/// An 8-team, full-placement tournament on two courts where QF1 and QF2 were won by side A
/// and SF1 has one point: then QF1's winner is flipped, so QF1 is held.
async fn held_bracket(s: &Store, pool: &SqlitePool) -> Competition {
    let (ev, _) = event(s, full(), 8, 2).await;
    let ev = start(s, &ev).await.unwrap();
    let (q1, q2) = (qf(pool, &ev, 1).await, qf(pool, &ev, 2).await);
    finish(s, &q1, Team::A).await;
    finish(s, &q2, Team::A).await;
    let sf1 = node(pool, &ev, SF1).await.5.unwrap();
    applied(score_point(s, &token(pool, &sf1).await).await);
    applied(typed(s, &q1, 2, 6).await);
    assert!(is_held(pool, &ev, QF1).await);
    ev
}

#[tokio::test]
async fn load_in_reads_started_and_winner() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q2, q3) = (
        qf(&pool, &ev, 1).await,
        qf(&pool, &ev, 2).await,
        qf(&pool, &ev, 3).await,
    );
    applied(score_point(&s, &token(&pool, &q1).await).await);
    let t2 = token(&pool, &q2).await;
    applied(score_point(&s, &t2).await);
    applied(undo(&s, &t2).await);
    finish(&s, &q3, Team::B).await;
    let mut conn = pool.acquire().await.unwrap();
    let b = load_in(&mut conn, &ev).await.unwrap().unwrap();
    assert_eq!(b.nodes.len(), 12);
    let n = |p: Pos| b.node(p).unwrap().clone();
    assert!(n(QF1).started && n(QF1).winner.is_none());
    assert!(!n(QF2).started, "an undone point does not start a match");
    assert!(n(QF3).started);
    assert_eq!(n(QF3).winner, Some(Side::B));
    assert!(!n(QF4).started && n(QF4).winner.is_none());
    assert_eq!(n(QF1).match_id.as_deref(), Some(q1.as_str()));
    assert_eq!(n(SF1).match_id, None);
    assert!(!n(SF1).started);
    drop(conn);
    // An event with no bracket rows has no bracket.
    let (other, _) = event(&s, None, 4, 1).await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(load_in(&mut conn, &other).await.unwrap().is_none());
}

#[tokio::test]
async fn withdraw_handles_every_court_case() {
    let (s, pool) = setup().await;

    // The current match of a court, unfinished: the court has no current match after.
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let q1 = qf(&pool, &ev, 1).await;
    withdraw(&s, &ev, &q1).await.unwrap();
    assert!(!exists(&pool, &q1).await);
    assert_eq!(
        node(&pool, &ev, QF1).await.5,
        None,
        "the cascade clears match_id"
    );
    assert_eq!(courts(&pool, &ev).await[0].current_match_id, None);
    check_rows(&pool, &ev).await;

    // The finished current match: the court moves on to its queued match.
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q3) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 3).await);
    finish(&s, &q1, Team::A).await;
    assert_eq!(
        court_of(&pool, &q3).await,
        Some(courts(&pool, &ev).await[0].id.clone())
    );
    withdraw(&s, &ev, &q1).await.unwrap();
    assert!(!exists(&pool, &q1).await);
    assert_eq!(node(&pool, &ev, QF1).await.5, None);
    assert_eq!(
        courts(&pool, &ev).await[0].current_match_id.as_deref(),
        Some(q3.as_str())
    );

    // A queued match: the court keeps its current match and has nothing queued.
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q3) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 3).await);
    finish(&s, &q1, Team::A).await;
    withdraw(&s, &ev, &q3).await.unwrap();
    assert!(!exists(&pool, &q3).await);
    assert_eq!(node(&pool, &ev, QF3).await.5, None);
    let c1 = courts(&pool, &ev).await[0].clone();
    assert_eq!(c1.current_match_id.as_deref(), Some(q1.as_str()));
    let state = court_state_by_token(&s, &c1.scorer_token)
        .await
        .unwrap()
        .unwrap()
        .state;
    assert!(state.next.is_none());

    // A match on no court: nothing else changes.
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let q4 = qf(&pool, &ev, 4).await;
    let before = courts(&pool, &ev).await;
    withdraw(&s, &ev, &q4).await.unwrap();
    assert!(!exists(&pool, &q4).await);
    assert_eq!(node(&pool, &ev, QF4).await.5, None);
    assert_eq!(courts(&pool, &ev).await, before);

    // A finished match that is no longer current: the court keeps its current match.
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q3) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 3).await);
    finish(&s, &q1, Team::A).await;
    let c1 = courts(&pool, &ev).await[0].id.clone();
    s.advance_court(A, &ev.id, &c1).await.unwrap();
    withdraw(&s, &ev, &q1).await.unwrap();
    assert!(!exists(&pool, &q1).await);
    assert_eq!(node(&pool, &ev, QF1).await.5, None);
    assert_eq!(
        courts(&pool, &ev).await[0].current_match_id.as_deref(),
        Some(q3.as_str())
    );

    // Another event's match is not this event's to withdraw.
    let (other, _) = event(&s, full(), 8, 1).await;
    let other = start(&s, &other).await.unwrap();
    let theirs = qf(&pool, &other, 1).await;
    let err = withdraw(&s, &ev, &theirs).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    assert!(exists(&pool, &theirs).await);
}

#[tokio::test]
async fn apply_plan_carries_out_every_part() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (r1, r2) = (node(&pool, &ev, QF1).await, node(&pool, &ev, QF2).await);
    let q4 = qf(&pool, &ev, 4).await;
    let mut conn = pool.acquire().await.unwrap();
    let bracket = load_in(&mut conn, &ev).await.unwrap().unwrap();
    drop(conn);
    let plan = Plan {
        set: vec![
            (SF1, Side::A, Some(side(&r1, Team::A))),
            (SF1, Side::B, Some(side(&r2, Team::A))),
        ],
        withdraw: vec![q4.clone()],
        create: vec![SF1],
        held: vec![(QF1, true)],
    };
    let mut tx = s.begin().await.unwrap();
    let withdrawn = apply_plan_in(&mut tx, &ev, &bracket, &plan, NOW)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(withdrawn, vec![q4.clone()]);
    assert!(!exists(&pool, &q4).await);
    assert_eq!(node(&pool, &ev, QF4).await.5, None);
    let sf1 = node(&pool, &ev, SF1).await;
    assert_eq!(sf1.3, Some(side(&r1, Team::A)));
    assert_eq!(sf1.4, Some(side(&r2, Team::A)));
    let m = sf1.5.unwrap();
    assert_eq!(label_of(&pool, &m).await, "Semi-final 1");
    assert!(is_held(&pool, &ev, QF1).await);
    // Both courts are busy, so the new match waits.
    assert_eq!(court_of(&pool, &m).await, None);
    check_rows(&pool, &ev).await;
}

#[tokio::test]
async fn results_move_teams_on() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (r1, r2) = (node(&pool, &ev, QF1).await, node(&pool, &ev, QF2).await);
    finish(&s, &r1.5.clone().unwrap(), Team::A).await;
    assert_eq!(node(&pool, &ev, SF1).await.5, None, "SF1 waits for QF2");
    finish(&s, &r2.5.clone().unwrap(), Team::B).await;
    let sf1 = node(&pool, &ev, SF1).await;
    assert_eq!(sf1.3, Some(side(&r1, Team::A)));
    assert_eq!(sf1.4, Some(side(&r2, Team::B)));
    assert_eq!(label_of(&pool, &sf1.5.unwrap()).await, "Semi-final 1");
    let ps1 = node(&pool, &ev, PS1).await;
    assert_eq!(ps1.3, Some(side(&r1, Team::B)));
    assert_eq!(ps1.4, Some(side(&r2, Team::A)));
    assert_eq!(
        label_of(&pool, &ps1.5.unwrap()).await,
        "Placement semi 1 (5th–8th)"
    );
    check_rows(&pool, &ev).await;
}

#[tokio::test]
async fn finished_court_gets_next_match() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q3) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 3).await);
    finish(&s, &q1, Team::A).await;
    let c1 = courts(&pool, &ev).await[0].clone();
    let state = court_state_by_token(&s, &c1.scorer_token)
        .await
        .unwrap()
        .unwrap()
        .state;
    assert_eq!(state.current.map(|m| m.id), Some(q1.clone()));
    assert_eq!(state.next.map(|m| m.id), Some(q3.clone()));
    let moved = s.advance_court_by_token(&c1.scorer_token).await.unwrap();
    assert_eq!(moved.current.as_deref(), Some(q3.as_str()));
    assert_eq!(
        courts(&pool, &ev).await[0].current_match_id.as_deref(),
        Some(q3.as_str())
    );
}

/// Whether the bracket match is on a court.
async fn on_court(pool: &SqlitePool, p: &Row6) -> bool {
    court_of(pool, p.5.as_ref().unwrap()).await.is_some()
}

#[tokio::test]
async fn final_waits_for_every_other_match() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    for n in 1..=4 {
        let q = qf(&pool, &ev, n).await;
        finish(&s, &q, Team::A).await;
    }
    // Both semi-finals finish while the placement semis are unplayed: the final is made but
    // the 5th/7th-place matches do not exist yet, so it stays off the courts.
    for p in [pos(1, 4, 0), pos(1, 4, 1)] {
        let m = node(&pool, &ev, p).await.5.unwrap();
        finish(&s, &m, Team::A).await;
    }
    let f = node(&pool, &ev, FINAL).await;
    assert!(f.5.is_some(), "the final is made");
    assert!(
        !on_court(&pool, &f).await,
        "the final waits: placement semis unplayed"
    );
    assert!(node(&pool, &ev, pos(5, 2, 0)).await.5.is_none());
    // Play the rest, oldest first, keeping the final off its court until every other bracket
    // match is on a court or finished.
    loop {
        fill(&s, &ev).await;
        let all = rows(&pool, &ev).await;
        let mut others_placed = true;
        for r in all.iter().filter(|r| (r.0, r.1) != (1, 2)) {
            match &r.5 {
                None => others_placed = false,
                Some(m) => {
                    let done: i64 =
                        sqlx::query_scalar("SELECT COUNT(*) FROM match_results WHERE match_id = ?")
                            .bind(m)
                            .fetch_one(&pool)
                            .await
                            .unwrap();
                    if done == 0 && court_of(&pool, m).await.is_none() {
                        others_placed = false;
                    }
                }
            }
        }
        let f = node(&pool, &ev, FINAL).await;
        if !others_placed {
            assert!(
                !on_court(&pool, &f).await,
                "the final went onto a court early"
            );
        }
        let next = unfinished(&pool, &ev)
            .await
            .into_iter()
            .find(|m| Some(m) != f.5.as_ref());
        match next {
            Some(m) => finish(&s, &m, Team::A).await,
            None => break,
        }
    }
    let f = node(&pool, &ev, FINAL).await;
    assert!(
        on_court(&pool, &f).await,
        "the final goes onto a free court at the end"
    );
}

#[tokio::test]
async fn two_courts_finish_together() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let mut q = Vec::new();
    for n in 1..=4 {
        q.push(qf(&pool, &ev, n).await);
    }
    finish(&s, &q[0], Team::A).await;
    finish(&s, &q[1], Team::A).await;
    let cs = courts(&pool, &ev).await;
    let all = assignments(&pool, &ev).await;
    let on = |court: &str| -> Vec<String> {
        all.iter()
            .filter(|a| a.1.as_deref() == Some(court))
            .map(|a| a.0.clone())
            .collect()
    };
    assert_eq!(on(&cs[0].id), vec![q[0].clone(), q[2].clone()]);
    assert_eq!(on(&cs[1].id), vec![q[1].clone(), q[3].clone()]);
    for p in [SF1, PS1] {
        let m = node(&pool, &ev, p).await.5.unwrap();
        assert_eq!(court_of(&pool, &m).await, None, "{p:?} waits on no court");
    }
}

#[tokio::test]
async fn court_undo_after_next_created_is_ok() {
    let (s, pool) = setup().await;
    // Without full placement QF1 and QF2 make only SF1; three courts, so SF1 gets a court of
    // its own while QF1 is still current on court 1.
    let (ev, _) = event(&s, None, 8, 3).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q2) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 2).await);
    let r2 = node(&pool, &ev, QF2).await;
    finish_by_scorer(&s, &q1).await;
    finish(&s, &q2, Team::A).await;
    let c2 = courts(&pool, &ev).await[1].clone();
    let old = node(&pool, &ev, SF1).await.5.unwrap();
    s.advance_court_by_token(&c2.scorer_token).await.unwrap();
    assert_eq!(
        courts(&pool, &ev).await[1].current_match_id.as_deref(),
        Some(old.as_str())
    );
    let q1_token = token(&pool, &q1).await;
    assert_eq!(
        courts(&pool, &ev).await[0].current_match_id.as_deref(),
        Some(q1.as_str())
    );

    applied(undo(&s, &q1_token).await);
    let sf1 = node(&pool, &ev, SF1).await;
    assert_eq!(sf1.3, None, "slot A is empty again");
    assert_eq!(sf1.4, Some(side(&r2, Team::A)));
    assert_eq!(sf1.5, None);
    assert!(!exists(&pool, &old).await, "SF1 was withdrawn");
    assert_eq!(courts(&pool, &ev).await[1].current_match_id, None);
    check_rows(&pool, &ev).await;

    applied(score_point(&s, &q1_token).await);
    let r1 = node(&pool, &ev, QF1).await;
    let sf1 = node(&pool, &ev, SF1).await;
    let new = sf1.5.clone().unwrap();
    assert_ne!(new, old);
    assert_eq!(sf1.3, Some(side(&r1, Team::A)));
    assert_eq!(sf1.4, Some(side(&r2, Team::A)));
    assert_eq!(court_of(&pool, &new).await, Some(c2.id.clone()));
    check_rows(&pool, &ev).await;
}

#[tokio::test]
async fn correction_before_next_started_remakes() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (r1, r2) = (node(&pool, &ev, QF1).await, node(&pool, &ev, QF2).await);
    let (q1, q2) = (r1.5.clone().unwrap(), r2.5.clone().unwrap());
    finish(&s, &q1, Team::A).await;
    finish(&s, &q2, Team::A).await;
    let old_sf1 = node(&pool, &ev, SF1).await.5.unwrap();
    let old_ps1 = node(&pool, &ev, PS1).await.5.unwrap();
    applied(typed(&s, &q1, 2, 6).await);
    assert!(!is_held(&pool, &ev, QF1).await);
    let sf1 = node(&pool, &ev, SF1).await;
    assert_ne!(sf1.5.as_deref(), Some(old_sf1.as_str()));
    assert!(sf1.5.is_some());
    assert_eq!(sf1.3, Some(side(&r1, Team::B)));
    assert_eq!(sf1.4, Some(side(&r2, Team::A)));
    let ps1 = node(&pool, &ev, PS1).await;
    assert_ne!(ps1.5.as_deref(), Some(old_ps1.as_str()));
    assert_eq!(ps1.3, Some(side(&r1, Team::A)));
    assert_eq!(ps1.4, Some(side(&r2, Team::B)));
    assert!(!exists(&pool, &old_sf1).await);
    assert!(!exists(&pool, &old_ps1).await);
    check_rows(&pool, &ev).await;
}

#[tokio::test]
async fn correction_after_next_started_is_held() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let q1 = qf(&pool, &ev, 1).await;
    let q2 = qf(&pool, &ev, 2).await;
    finish(&s, &q1, Team::A).await;
    finish(&s, &q2, Team::A).await;
    let sf1 = node(&pool, &ev, SF1).await;
    let ps1 = node(&pool, &ev, PS1).await;
    let sf1_id = sf1.5.clone().unwrap();
    applied(score_point(&s, &token(&pool, &sf1_id).await).await);

    let outcome = typed(&s, &q1, 2, 6).await;
    assert!(matches!(outcome, CommandOutcome::Applied(_)), "{outcome:?}");
    assert!(is_held(&pool, &ev, QF1).await);
    assert_eq!(
        node(&pool, &ev, SF1).await,
        sf1,
        "SF1 keeps its match and teams"
    );
    assert_eq!(
        node(&pool, &ev, PS1).await,
        ps1,
        "nothing downstream changes"
    );
    assert_eq!(
        effective_events(&s, &sf1_id).await,
        1,
        "SF1 keeps its point"
    );
    check_rows(&pool, &ev).await;
}

#[tokio::test]
async fn correction_back_clears_hold() {
    let (s, pool) = setup().await;
    let ev = held_bracket(&s, &pool).await;
    let (sf1, ps1) = (node(&pool, &ev, SF1).await, node(&pool, &ev, PS1).await);
    let q1 = qf(&pool, &ev, 1).await;
    applied(typed(&s, &q1, 6, 2).await);
    assert!(!is_held(&pool, &ev, QF1).await);
    assert_eq!(node(&pool, &ev, SF1).await, sf1);
    assert_eq!(node(&pool, &ev, PS1).await, ps1);
}

#[tokio::test]
async fn bracket_never_refuses_a_point() {
    let (s, pool) = setup().await;
    let ev = held_bracket(&s, &pool).await;
    let mut done = HashSet::new();
    loop {
        let next = unfinished(&pool, &ev)
            .await
            .into_iter()
            .find(|m| !done.contains(m));
        let Some(m) = next else { break };
        done.insert(m.clone());
        let t = token(&pool, &m).await;
        applied(score_point(&s, &t).await);
        applied(undo(&s, &t).await);
        applied(score_point(&s, &t).await);
        applied(typed(&s, &m, 6, 0).await);
        check_rows(&pool, &ev).await;
    }
    assert!(done.len() >= 4, "{done:?}");
    assert!(is_held(&pool, &ev, QF1).await);
}

#[tokio::test]
async fn finish_needs_complete_bracket() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let err = finish_event(&s, &ev).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("bracket_unfinished")),
        "{err:?}"
    );
    let ev = held_bracket(&s, &pool).await;
    let err = finish_event(&s, &ev).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("bracket_needs_decision")),
        "{err:?}"
    );
    let got = s.competition(A, &ev.id).await.unwrap().unwrap();
    assert_eq!(got.status, Status::Running);
    assert!(placing_rows(&pool, &ev).await.is_empty());
}

#[tokio::test]
async fn full_run_writes_placings() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    assert_eq!(
        rows(&pool, &ev)
            .await
            .iter()
            .filter(|r| r.5.is_some())
            .count(),
        12
    );
    let done = finish_event(&s, &ev).await.unwrap();
    assert_eq!(done.status, Status::Finished);
    let written = placing_rows(&pool, &ev).await;
    assert_eq!(written.len(), 16);
    assert!(written.iter().all(|r| r.3 == 8 && r.4 == FINISHED_AT));
    let mut places: Vec<(u32, u32)> = team_places(&pool, &ev).await.into_values().collect();
    places.sort_unstable();
    assert_eq!(places, (1..=8).map(|p| (p, p)).collect::<Vec<_>>());
    // Side A won every match: the final's side A is 1st.
    let fin = node(&pool, &ev, FINAL).await;
    let tp = team_places(&pool, &ev).await;
    assert_eq!(tp[&side(&fin, Team::A)], (1, 1));
    assert_eq!(tp[&side(&fin, Team::B)], (2, 2));
}

#[tokio::test]
async fn off_mode_shares_fifth() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, None, 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    finish_event(&s, &ev).await.unwrap();
    assert_eq!(placing_rows(&pool, &ev).await.len(), 16);
    let tp = team_places(&pool, &ev).await;
    let mut places: Vec<(u32, u32)> = tp.values().copied().collect();
    places.sort_unstable();
    assert_eq!(
        places,
        [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 8),
            (5, 8),
            (5, 8),
            (5, 8)
        ]
    );
    // The quarter-final losers are the ones sharing 5th.
    for n in 1..=4 {
        let r = node(&pool, &ev, pos(1, 8, n - 1)).await;
        assert_eq!(tp[&side(&r, Team::B)], (5, 8));
    }
}

#[tokio::test]
async fn final_correction_after_finish_rewrites_placings() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    finish_event(&s, &ev).await.unwrap();
    let before = team_places(&pool, &ev).await;
    let fin = node(&pool, &ev, FINAL).await;
    let (a, b) = (side(&fin, Team::A), side(&fin, Team::B));
    applied(typed(&s, &fin.5.clone().unwrap(), 3, 6).await);
    let after = team_places(&pool, &ev).await;
    assert_eq!((after[&a], after[&b]), ((2, 2), (1, 1)));
    for (team, place) in &before {
        if *team != a && *team != b {
            assert_eq!(after[team], *place);
        }
    }
    let written = placing_rows(&pool, &ev).await;
    assert_eq!(written.len(), 16);
    assert!(
        written.iter().all(|r| r.4 == FINISHED_AT),
        "finished_at is kept"
    );
    let got = s.competition(A, &ev.id).await.unwrap().unwrap();
    assert_eq!(got.finished_at, Some(FINISHED_AT));
    assert_eq!(got.status, Status::Finished);
}

#[tokio::test]
async fn early_correction_after_finish_is_held() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    finish_event(&s, &ev).await.unwrap();
    let before = placing_rows(&pool, &ev).await;
    let bracket_before = rows(&pool, &ev).await;
    let q1 = qf(&pool, &ev, 1).await;
    applied(typed(&s, &q1, 0, 6).await);
    assert!(is_held(&pool, &ev, QF1).await);
    assert_eq!(placing_rows(&pool, &ev).await, before);
    assert_eq!(rows(&pool, &ev).await, bracket_before);
}

#[tokio::test]
async fn finished_event_never_withdraws() {
    // After Finish no match can be made or taken off a court, so a plan that would withdraw a
    // match becomes a hold (decision 8). Every bracket match has a result by then, so this
    // state is forced: SF1's log and result are removed by hand to make it "not started".
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    finish_event(&s, &ev).await.unwrap();
    let before = placing_rows(&pool, &ev).await;
    let sf1 = node(&pool, &ev, SF1).await;
    let sf1_id = sf1.5.clone().unwrap();
    for sql in [
        "DELETE FROM events WHERE match_id = ?",
        "DELETE FROM match_results WHERE match_id = ?",
        "DELETE FROM command_results WHERE match_id = ?",
    ] {
        sqlx::query(sql).bind(&sf1_id).execute(&pool).await.unwrap();
    }
    let q1 = qf(&pool, &ev, 1).await;
    applied(typed(&s, &q1, 0, 6).await);
    assert!(is_held(&pool, &ev, QF1).await);
    assert!(exists(&pool, &sf1_id).await);
    assert_eq!(node(&pool, &ev, SF1).await, sf1);
    assert_eq!(placing_rows(&pool, &ev).await, before);
}

#[tokio::test]
async fn bracket_rows_match_their_matches() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    check_rows(&pool, &ev).await;
    play_all(&s, &pool, &ev, true).await;
    finish_event(&s, &ev).await.unwrap();
    check_rows(&pool, &ev).await;
}

const SF2: Pos = pos(1, 4, 1);
const THIRD: Pos = pos(3, 2, 0);

async fn decide(s: &Store, ev: &Competition, match_id: &str, choice: Choice) -> Vec<String> {
    s.decide_bracket(A, &ev.id, match_id, choice, NOW)
        .await
        .unwrap()
}

#[tokio::test]
async fn replay_resolves_holds_it_unblocked() {
    // SF2 is held because the final started; QF1 is held because SF1 finished. Replaying QF1
    // withdraws SF1 and the final, so SF2's hold resolves: its corrected winner and loser go
    // to the final and the 3rd-place match, and no match is made with its stale team.
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, full(), 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    for n in 1..=4 {
        finish(&s, &qf(&pool, &ev, n).await, Team::A).await;
    }
    let sf1 = node(&pool, &ev, SF1).await.5.unwrap();
    let sf2 = node(&pool, &ev, SF2).await.5.unwrap();
    finish(&s, &sf1, Team::A).await;
    finish(&s, &sf2, Team::A).await;
    let fin = node(&pool, &ev, FINAL).await.5.unwrap();
    applied(score_point(&s, &token(&pool, &fin).await).await);
    applied(typed(&s, &sf2, 1, 6).await);
    assert!(is_held(&pool, &ev, SF2).await);
    let q1 = qf(&pool, &ev, 1).await;
    applied(typed(&s, &q1, 1, 6).await);
    assert!(is_held(&pool, &ev, QF1).await);

    let withdrawn = decide(&s, &ev, &q1, Choice::Replay).await;
    assert!(withdrawn.contains(&sf1) && withdrawn.contains(&fin));
    assert!(!exists(&pool, &fin).await);
    assert!(!is_held(&pool, &ev, QF1).await);
    assert!(!is_held(&pool, &ev, SF2).await, "SF2's hold resolved");
    let sf2_row = node(&pool, &ev, SF2).await;
    let (sf2_winner, sf2_loser) = (side(&sf2_row, Team::B), side(&sf2_row, Team::A));
    let final_row = node(&pool, &ev, FINAL).await;
    assert_eq!(
        (final_row.3.clone(), final_row.4.clone()),
        (None, Some(sf2_winner.clone()))
    );
    assert!(final_row.5.is_none());
    let third = node(&pool, &ev, THIRD).await;
    assert_eq!((third.3.clone(), third.4.clone()), (None, Some(sf2_loser)));
    check_rows(&pool, &ev).await;

    // The new SF1 finishes: the final is made with SF2's corrected winner.
    let new_sf1 = node(&pool, &ev, SF1).await.5.unwrap();
    finish(&s, &new_sf1, Team::A).await;
    let final_row = node(&pool, &ev, FINAL).await;
    assert!(final_row.5.is_some());
    assert_eq!(side(&final_row, Team::B), sf2_winner);
    check_rows(&pool, &ev).await;
    play_all(&s, &pool, &ev, true).await;
    finish_event(&s, &ev).await.unwrap();
}

#[tokio::test]
async fn keep_as_played_places_follow_the_bracket() {
    // Off mode: QF1 is corrected after SF1 started, then kept as played. QF1's side A team
    // played on, so its side B team shares 5th, and Finish gets places that tile.
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, None, 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    let (q1, q2) = (qf(&pool, &ev, 1).await, qf(&pool, &ev, 2).await);
    finish(&s, &q1, Team::A).await;
    finish(&s, &q2, Team::A).await;
    let sf1 = node(&pool, &ev, SF1).await.5.unwrap();
    applied(score_point(&s, &token(&pool, &sf1).await).await);
    applied(typed(&s, &q1, 2, 6).await);
    assert!(is_held(&pool, &ev, QF1).await);
    let withdrawn = decide(&s, &ev, &q1, Choice::Keep).await;
    assert!(withdrawn.is_empty());
    assert!(!is_held(&pool, &ev, QF1).await);
    assert!(exists(&pool, &sf1).await);
    play_all(&s, &pool, &ev, true).await;
    finish_event(&s, &ev).await.unwrap();
    let qf1 = node(&pool, &ev, QF1).await;
    let tp = team_places(&pool, &ev).await;
    assert_eq!(tp.len(), 8);
    assert_eq!(tp[&side(&qf1, Team::B)], (5, 8));
    assert_ne!(tp[&side(&qf1, Team::A)], (5, 8));

    // A final correction after Finish rewrites the places.
    let fin = node(&pool, &ev, FINAL).await;
    let (a, b) = (side(&fin, Team::A), side(&fin, Team::B));
    applied(typed(&s, &fin.5.clone().unwrap(), 3, 6).await);
    let after = team_places(&pool, &ev).await;
    assert_eq!((after[&a], after[&b]), ((2, 2), (1, 1)));
    assert_eq!(after[&side(&qf1, Team::B)], (5, 8));
}

#[tokio::test]
async fn keep_after_finish_rewrites_placings() {
    let (s, pool) = setup().await;
    let (ev, _) = event(&s, None, 8, 2).await;
    let ev = start(&s, &ev).await.unwrap();
    play_all(&s, &pool, &ev, false).await;
    finish_event(&s, &ev).await.unwrap();
    let before = team_places(&pool, &ev).await;
    let q1 = qf(&pool, &ev, 1).await;
    applied(typed(&s, &q1, 0, 6).await);
    assert!(is_held(&pool, &ev, QF1).await);
    sqlx::query("DELETE FROM placings WHERE event_id = ?")
        .bind(&ev.id)
        .execute(&pool)
        .await
        .unwrap();
    let ev = s.competition(A, &ev.id).await.unwrap().unwrap();
    let replay = s.decide_bracket(A, &ev.id, &q1, Choice::Replay, NOW).await;
    assert!(
        matches!(replay, Err(StoreError::Conflict("event_finished"))),
        "{replay:?}"
    );
    decide(&s, &ev, &q1, Choice::Keep).await;
    assert!(!is_held(&pool, &ev, QF1).await);
    // Kept as played: QF1's side A team went on, so the places are as before.
    assert_eq!(team_places(&pool, &ev).await, before);
    let written = placing_rows(&pool, &ev).await;
    assert_eq!(written.len(), 16);
    assert!(written.iter().all(|r| r.4 == FINISHED_AT));
}
