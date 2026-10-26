#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::{HashMap, HashSet};

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, FinalRequest, apply_command, record_final,
};
use padel_score_server::competitions::courts::{EventMatchRow, event_matches_in};
use padel_score_server::competitions::non_stop::clock::{
    COUNT_IN_MS, ClockAction, any_running_non_stop_in, clear_time_up_in, due_go_in, due_slots_in,
    mark_done_in, next_deadline_in, pending_time_up_in, went_in,
};
use padel_score_server::competitions::non_stop::standings::{Outcome, places_of, standings};
use padel_score_server::competitions::{Audience, Competition, Kind, NewCompetition, kinds};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DecidingSet, DeuceRule, FinalScore, Format, SetRules, SetsFormat, Team};
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};

const A: &str = "user-a";
const NOW: i64 = 1_000;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn timed() -> Format {
    Format::Timed {
        deuce: DeuceRule::GoldenPoint,
        draws_allowed: false,
    }
}

fn timed_draws() -> Format {
    Format::Timed {
        deuce: DeuceRule::GoldenPoint,
        draws_allowed: true,
    }
}

async fn create(s: &Store, settings: Option<Value>) -> Result<Competition, StoreError> {
    create_fmt(s, timed(), settings).await
}

async fn create_fmt(
    s: &Store,
    format: Format,
    settings: Option<Value>,
) -> Result<Competition, StoreError> {
    s.create_competition(
        A,
        NewCompetition {
            name: "Americano".into(),
            kind: Kind::NonStop,
            format,
            settings,
        },
        10,
    )
    .await
}

/// A non-stop in `setup` with `n_teams` teams and `n_courts` courts.
async fn event_with(
    s: &Store,
    settings: Option<Value>,
    n_teams: usize,
    n_courts: usize,
) -> (Competition, Vec<String>, Vec<String>) {
    event_fmt(s, timed(), settings, n_teams, n_courts).await
}

async fn event_fmt(
    s: &Store,
    format: Format,
    settings: Option<Value>,
    n_teams: usize,
    n_courts: usize,
) -> (Competition, Vec<String>, Vec<String>) {
    let ev = create_fmt(s, format, settings).await.unwrap();
    let mut teams = Vec::new();
    for i in 0..n_teams {
        let p1 = s
            .create_or_get_player(A, &format!("P{i}a"), 1)
            .await
            .unwrap()
            .0
            .id;
        let p2 = s
            .create_or_get_player(A, &format!("P{i}b"), 1)
            .await
            .unwrap()
            .0
            .id;
        teams.push(s.add_team(A, &ev.id, [p1, p2], None, 10).await.unwrap().id);
    }
    let mut courts = Vec::new();
    for _ in 0..n_courts {
        courts.push(s.add_court(A, &ev.id, None).await.unwrap().id);
    }
    (ev, teams, courts)
}

async fn matches_of(pool: &SqlitePool, ev: &Competition) -> Vec<EventMatchRow> {
    let mut conn = pool.acquire().await.unwrap();
    event_matches_in(&mut conn, &ev.id).await.unwrap()
}

fn pair(m: &EventMatchRow) -> (String, String) {
    let (a, b) = (m.team_a_id.clone(), m.team_b_id.clone());
    if a < b { (a, b) } else { (b, a) }
}

#[tokio::test]
async fn settings_default_and_bounds() {
    let (s, _) = setup().await;
    let ev = create(&s, None).await.unwrap();
    assert_eq!(
        ev.settings,
        json!({"match_minutes": 15, "legs": 1, "order": "listed"})
    );
    assert_eq!(
        create(&s, Some(json!({}))).await.unwrap().settings,
        ev.settings
    );
    for bad in [
        json!({"match_minutes": 0}),
        json!({"match_minutes": 61}),
        json!({"legs": 0}),
        json!({"legs": 4}),
        json!({"x": 1}),
        json!([]),
        json!(3),
        json!({"order": "seeded"}),
    ] {
        let err = create(&s, Some(bad.clone())).await.unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("settings")),
            "{bad}: {err:?}"
        );
    }
    assert_eq!(
        create(&s, Some(json!({"legs": 2}))).await.unwrap().settings,
        json!({"match_minutes": 15, "legs": 2, "order": "listed"})
    );
    assert_eq!(
        create(
            &s,
            Some(json!({"match_minutes": 60, "legs": 3, "order": "random"}))
        )
        .await
        .unwrap()
        .settings,
        json!({"match_minutes": 60, "legs": 3, "order": "random"})
    );
}

#[tokio::test]
async fn random_order_still_meets_everyone() {
    let (s, pool) = setup().await;
    let (ev, teams, _) = event_with(&s, Some(json!({"order": "random"})), 8, 2).await;
    s.start_competition(A, &ev.id, NOW).await.unwrap();
    let ms = matches_of(&pool, &ev).await;
    assert_eq!(ms.len(), 28);
    let pairs: HashSet<_> = ms.iter().map(pair).collect();
    assert_eq!(pairs.len(), 28);
    let all: HashSet<_> = teams.iter().cloned().collect();
    assert!(
        ms.iter()
            .all(|m| all.contains(&m.team_a_id) && all.contains(&m.team_b_id))
    );
}

#[tokio::test]
async fn odd_team_count_cannot_start() {
    let (s, pool) = setup().await;
    let (ev, _, _) = event_with(&s, None, 5, 1).await;
    let err = s.start_competition(A, &ev.id, NOW).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict("odd_teams")), "{err:?}");
    let status: String = sqlx::query_scalar("SELECT status FROM competitions WHERE id = ?")
        .bind(&ev.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "setup");
}

#[tokio::test]
async fn schedule_too_big_cannot_start() {
    let (s, _) = setup().await;
    let (ev, _, _) = event_with(&s, Some(json!({"legs": 2})), 18, 1).await;
    let err = s.start_competition(A, &ev.id, NOW).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("schedule_too_big")),
        "{err:?}"
    );
}

#[tokio::test]
async fn start_builds_the_whole_schedule() {
    let (s, pool) = setup().await;
    let (ev, _, courts) = event_with(&s, None, 8, 2).await;
    s.start_competition(A, &ev.id, NOW).await.unwrap();
    let ms = matches_of(&pool, &ev).await;
    assert_eq!(ms.len(), 28);
    assert_eq!(ms.iter().map(pair).collect::<HashSet<_>>().len(), 28);
    assert_eq!(ms[0].label, "Round 1, match 1");
    assert_eq!(ms[1].label, "Round 1, match 2");
    assert_eq!(ms[2].label, "Round 1, match 3");
    assert_eq!(ms[27].label, "Round 7, match 4");

    let slots = sqlx::query(
        "SELECT slot, round, wave, state, duration_ms, remaining_ms FROM non_stop_slots \
         WHERE event_id = ? ORDER BY slot",
    )
    .bind(&ev.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(slots.len(), 14);
    for (i, r) in slots.iter().enumerate() {
        assert_eq!(r.get::<i64, _>("slot") as usize, i + 1);
        assert_eq!(r.get::<i64, _>("round") as usize, i / 2 + 1);
        assert_eq!(r.get::<i64, _>("wave") as usize, i % 2 + 1);
        assert_eq!(r.get::<String, _>("state"), "ready");
        assert_eq!(r.get::<i64, _>("duration_ms"), 900_000);
        assert_eq!(r.get::<i64, _>("remaining_ms"), 900_000);
    }

    // Each match's slot, in creation order: two per slot, with the leg 1-based.
    let rows = sqlx::query(
        "SELECT c.ordinal, n.slot, n.leg, n.needs_time_up FROM non_stop_matches n \
         JOIN competition_matches c ON c.match_id = n.match_id WHERE n.event_id = ? \
         ORDER BY c.ordinal",
    )
    .bind(&ev.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 28);
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r.get::<i64, _>("slot") as usize, i / 2 + 1);
        assert_eq!(r.get::<i64, _>("leg"), 1);
        assert_eq!(r.get::<i64, _>("needs_time_up"), 0);
    }

    // Court queues are in play order; the current match is the slot-1 one.
    let mut by_court: HashMap<String, Vec<String>> = HashMap::new();
    let mut q = sqlx::query(
        "SELECT court_id, match_id FROM competition_matches WHERE event_id = ? \
         AND court_id IS NOT NULL ORDER BY court_seq",
    )
    .bind(&ev.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    for r in q.drain(..) {
        by_court
            .entry(r.get("court_id"))
            .or_default()
            .push(r.get("match_id"));
    }
    for (c, court) in courts.iter().enumerate() {
        let expected: Vec<String> = ms
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 2 == c)
            .map(|(_, m)| m.match_id.clone())
            .collect();
        assert_eq!(by_court[court], expected, "court {c}");
        let current: Option<String> =
            sqlx::query_scalar("SELECT current_match_id FROM courts WHERE id = ?")
                .bind(court)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(current.as_deref(), Some(expected[0].as_str()));
    }
}

#[tokio::test]
async fn two_legs_double_the_matches() {
    let (s, pool) = setup().await;
    let (ev, _, _) = event_with(&s, Some(json!({"legs": 2, "match_minutes": 10})), 4, 1).await;
    s.start_competition(A, &ev.id, NOW).await.unwrap();
    assert_eq!(matches_of(&pool, &ev).await.len(), 12);
    let slots: Vec<(i64, i64)> =
        sqlx::query_as("SELECT slot, duration_ms FROM non_stop_slots WHERE event_id = ?")
            .bind(&ev.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(slots.len(), 12);
    assert!(slots.iter().all(|(_, d)| *d == 600_000));
    let legs: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT leg FROM non_stop_matches WHERE event_id = ?")
            .bind(&ev.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(legs.len(), 2);
    assert!(legs.contains(&1) && legs.contains(&2));
}

// ---- Task 4: the clock, views, standings and places ----

/// Clock time (unix ms) for the clock tests.
const T: i64 = 1_800_000_000_000;

/// A started non-stop with `n_teams` teams and `n_courts` courts in `format`.
async fn started(
    s: &Store,
    format: Format,
    n_teams: usize,
    n_courts: usize,
) -> (Competition, Vec<String>, Vec<String>) {
    let (ev, teams, courts) = event_fmt(s, format, None, n_teams, n_courts).await;
    let ev = s.start_competition(A, &ev.id, NOW).await.unwrap();
    (ev, teams, courts)
}

async fn clock(
    s: &Store,
    ev: &Competition,
    action: ClockAction,
    slot: u32,
    now_ms: i64,
) -> Result<(), StoreError> {
    s.clock_action(A, &ev.id, action, slot, now_ms).await
}

fn conflict(r: Result<(), StoreError>, code: &str) {
    match r {
        Err(StoreError::Conflict(c)) if c == code => {}
        other => panic!("expected Conflict({code}), got {other:?}"),
    }
}

#[derive(Debug, PartialEq)]
struct SlotRow {
    state: String,
    remaining_ms: i64,
    starts_at_ms: Option<i64>,
    ends_at_ms: Option<i64>,
    done_at_ms: Option<i64>,
}

async fn slot_row(pool: &SqlitePool, ev: &Competition, slot: u32) -> SlotRow {
    let r = sqlx::query(
        "SELECT state, remaining_ms, starts_at_ms, ends_at_ms, done_at_ms FROM non_stop_slots \
         WHERE event_id = ? AND slot = ?",
    )
    .bind(&ev.id)
    .bind(slot)
    .fetch_one(pool)
    .await
    .unwrap();
    SlotRow {
        state: r.get("state"),
        remaining_ms: r.get("remaining_ms"),
        starts_at_ms: r.get("starts_at_ms"),
        ends_at_ms: r.get("ends_at_ms"),
        done_at_ms: r.get("done_at_ms"),
    }
}

/// The slot's matches in play order.
async fn slot_matches(pool: &SqlitePool, ev: &Competition, slot: u32) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT n.match_id FROM non_stop_matches n JOIN competition_matches c \
         ON c.match_id = n.match_id WHERE n.event_id = ? AND n.slot = ? ORDER BY c.ordinal",
    )
    .bind(&ev.id)
    .bind(slot)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn kind_data(pool: &SqlitePool, ev: &Competition, audience: Audience) -> Value {
    let ev = refreshed(pool, ev).await;
    let mut conn = pool.acquire().await.unwrap();
    kinds::kind_data(Kind::NonStop, &mut conn, &ev, audience)
        .await
        .unwrap()
        .expect("non-stop has kind data")
}

async fn refreshed(pool: &SqlitePool, ev: &Competition) -> Competition {
    let mut conn = pool.acquire().await.unwrap();
    padel_score_server::competitions::competition_in(&mut conn, A, &ev.id)
        .await
        .unwrap()
        .unwrap()
}

async fn type_result(s: &Store, match_id: &str, a: u16, b: u16, ot: Option<Team>) {
    let m = s.match_owned_by(match_id, A).await.unwrap().unwrap();
    let req = FinalRequest {
        client_id: format!("final:{match_id}:{a}:{b}"),
        base_revision: None,
        score: FinalScore::Timed {
            a,
            b,
            overtime_winner: ot,
        },
    };
    match record_final(s, &m, req, Authority::Owner, NOW)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(_) => {}
        other => panic!("expected Applied, got {other:?}"),
    }
}

async fn send(s: &Store, match_id: &str, client_id: &str, commands: Vec<Command>) {
    let m = s.match_owned_by(match_id, A).await.unwrap().unwrap();
    let req = CommandRequest {
        client_id: client_id.into(),
        batch: false,
        base_revision: 0,
        commands,
    };
    match apply_command(s, &m, req, NOW, Authority::Owner)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(_) => {}
        other => panic!("expected Applied, got {other:?}"),
    }
}

/// Golden-point games: four straight points each.
fn games(winners: &[Team]) -> Vec<Command> {
    winners
        .iter()
        .flat_map(|&team| std::iter::repeat_n(Command::Point { team }, 4))
        .collect()
}

async fn flags(pool: &SqlitePool, ev: &Competition, slot: u32) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT needs_time_up FROM non_stop_matches WHERE event_id = ? AND slot = ? \
         ORDER BY match_id",
    )
    .bind(&ev.id)
    .bind(slot)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn current_of(pool: &SqlitePool, court_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT current_match_id FROM courts WHERE id = ?")
        .bind(court_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The event's typed or played outcomes, as the standings see them.
async fn outcomes(pool: &SqlitePool, ev: &Competition) -> Vec<Outcome> {
    let rows = sqlx::query(
        "SELECT c.team_a_id, c.team_b_id, r.winner, r.score FROM match_results r \
         JOIN competition_matches c ON c.match_id = r.match_id WHERE c.event_id = ? \
         ORDER BY c.ordinal",
    )
    .bind(&ev.id)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.iter()
        .map(|r| {
            let score: Value = serde_json::from_str(&r.get::<String, _>("score")).unwrap();
            let g = &score["regulation_games"];
            Outcome {
                a: r.get("team_a_id"),
                b: r.get("team_b_id"),
                winner: match r.get::<Option<String>, _>("winner").as_deref() {
                    Some("a") => Some(Team::A),
                    Some("b") => Some(Team::B),
                    _ => None,
                },
                games: [g[0].as_u64().unwrap() as u16, g[1].as_u64().unwrap() as u16],
            }
        })
        .collect()
}

#[tokio::test]
async fn start_runs_the_first_slot() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    let row = slot_row(&pool, &ev, 1).await;
    assert_eq!(row.state, "running");
    assert_eq!(row.starts_at_ms, Some(T + 5_000));
    assert_eq!(row.ends_at_ms, Some(T + 905_000));
    assert_eq!(COUNT_IN_MS, 5_000);
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    assert_eq!(kd["clock"]["slot"], 1);
    assert_eq!(kd["clock"]["round"], 1);
    assert_eq!(kd["clock"]["wave"], 1);
    assert_eq!(kd["clock"]["state"], "running");
    assert_eq!(kd["clock"]["duration_ms"], 900_000);
    assert_eq!(kd["clock"]["starts_at_ms"], T + 5_000);
    assert_eq!(kd["clock"]["ends_at_ms"], T + 905_000);
    // The real clock is long before T, so this is the full time.
    assert_eq!(kd["clock"]["remaining_ms"], 900_000);
    assert_eq!(kd["rounds"], 7);
    assert_eq!(kd["waves"], 2);
    assert_eq!(
        kd["settings"],
        json!({"match_minutes": 15, "legs": 1, "order": "listed"})
    );
    let slots = kd["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 14);
    assert_eq!(slots[0]["state"], "running");
    assert_eq!(slots[1]["state"], "ready");
    assert_eq!(slots[1]["round"], 1);
    assert_eq!(slots[1]["wave"], 2);
    assert_eq!(
        slots[0]["matches"],
        json!(slot_matches(&pool, &ev, 1).await)
    );
}

#[tokio::test]
async fn ready_clock_is_the_lowest_ready_slot() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 4, 2).await;
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    assert_eq!(
        kd["clock"],
        json!({"slot": 1, "round": 1, "wave": 1, "state": "ready", "duration_ms": 900_000,
               "remaining_ms": 900_000, "starts_at_ms": null, "ends_at_ms": null})
    );
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    clock(&s, &ev, ClockAction::End, 1, T + 10_000)
        .await
        .unwrap();
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    assert_eq!(kd["clock"]["slot"], 2);
    assert_eq!(kd["clock"]["state"], "ready");
    assert_eq!(kd["slots"][0]["state"], "done");
}

#[tokio::test]
async fn stale_slot_action_is_refused() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    conflict(
        clock(&s, &ev, ClockAction::Start, 2, T).await,
        "not_next_slot",
    );
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    conflict(
        clock(&s, &ev, ClockAction::Start, 1, T + 1).await,
        "clock_busy",
    );
    conflict(
        clock(&s, &ev, ClockAction::Pause, 2, T + 1).await,
        "clock_not_running",
    );
    conflict(
        clock(&s, &ev, ClockAction::Resume, 1, T + 1).await,
        "clock_not_paused",
    );
    conflict(
        clock(&s, &ev, ClockAction::End, 2, T + 1).await,
        "clock_not_running",
    );
    conflict(
        clock(&s, &ev, ClockAction::Start, 99, T + 1).await,
        "clock_busy",
    );
    // Nothing changed.
    let row = slot_row(&pool, &ev, 1).await;
    assert_eq!(row.state, "running");
    assert_eq!(row.starts_at_ms, Some(T + 5_000));
    // Another owner's event is unknown.
    let err = s
        .clock_action("user-b", &ev.id, ClockAction::Pause, 1, T)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn clock_needs_a_running_event() {
    let (s, _) = setup().await;
    let (ev, _, _) = event_with(&s, None, 4, 1).await;
    conflict(
        clock(&s, &ev, ClockAction::Start, 1, T).await,
        "event_not_running",
    );
    let ev = s.start_competition(A, &ev.id, NOW).await.unwrap();
    s.finish_competition(A, &ev.id, None, NOW + 1)
        .await
        .unwrap();
    conflict(
        clock(&s, &ev, ClockAction::Start, 1, T).await,
        "event_not_running",
    );
}

#[tokio::test]
async fn only_one_clock_runs() {
    let (s, pool) = setup().await;
    let (ev, _, courts) = started(&s, timed(), 4, 2).await;
    let one = slot_matches(&pool, &ev, 1).await;
    let two = slot_matches(&pool, &ev, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    type_result(&s, &one[0], 5, 3, None).await;
    clock(&s, &ev, ClockAction::End, 1, T + 60_000)
        .await
        .unwrap();
    conflict(
        clock(&s, &ev, ClockAction::Start, 2, T + 70_000).await,
        "previous_unfinished",
    );
    assert_eq!(slot_row(&pool, &ev, 2).await.state, "ready");
    type_result(&s, &one[1], 2, 4, None).await;
    clock(&s, &ev, ClockAction::Start, 2, T + 80_000)
        .await
        .unwrap();
    assert_eq!(slot_row(&pool, &ev, 2).await.state, "running");
    let mut current = Vec::new();
    for c in &courts {
        current.push(current_of(&pool, c).await.unwrap());
    }
    current.sort();
    let mut expected = two.clone();
    expected.sort();
    assert_eq!(current, expected);
}

#[tokio::test]
async fn pause_keeps_time_left() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    clock(&s, &ev, ClockAction::Pause, 1, T + 65_000)
        .await
        .unwrap();
    assert_eq!(
        slot_row(&pool, &ev, 1).await,
        SlotRow {
            state: "paused".into(),
            remaining_ms: 840_000,
            starts_at_ms: None,
            ends_at_ms: None,
            done_at_ms: None,
        }
    );
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    assert_eq!(kd["clock"]["state"], "paused");
    assert_eq!(kd["clock"]["remaining_ms"], 840_000);
    conflict(
        clock(&s, &ev, ClockAction::Start, 2, T + 70_000).await,
        "clock_busy",
    );
    conflict(
        clock(&s, &ev, ClockAction::Pause, 1, T + 70_000).await,
        "clock_not_running",
    );
    clock(&s, &ev, ClockAction::Resume, 1, T + 120_000)
        .await
        .unwrap();
    let row = slot_row(&pool, &ev, 1).await;
    assert_eq!(row.state, "running");
    assert_eq!(row.starts_at_ms, Some(T + 125_000));
    assert_eq!(row.ends_at_ms, Some(T + 965_000));
}

#[tokio::test]
async fn pause_in_count_in_keeps_full_time() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    clock(&s, &ev, ClockAction::Pause, 1, T + 2_000)
        .await
        .unwrap();
    assert_eq!(slot_row(&pool, &ev, 1).await.remaining_ms, 900_000);
    // Resumed at T + 10 000, so it ends at T + 915 000.
    clock(&s, &ev, ClockAction::Resume, 1, T + 10_000)
        .await
        .unwrap();
}

#[tokio::test]
async fn pause_after_time_is_over_is_refused() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    // Time ran out at T + 905 000 and the runner has not called it yet.
    conflict(
        clock(&s, &ev, ClockAction::Pause, 1, T + 905_000).await,
        "clock_not_running",
    );
    let row = slot_row(&pool, &ev, 1).await;
    assert_eq!(row.state, "running");
    assert_eq!(row.ends_at_ms, Some(T + 905_000));
}

#[tokio::test]
async fn idle_court_waits_for_its_own_slot() {
    // 6 teams on 2 courts: 3 matches a round, so wave 1 uses courts 1-2 and wave 2 court 1.
    let (s, pool) = setup().await;
    let (ev, _, courts) = started(&s, timed(), 6, 2).await;
    let one = slot_matches(&pool, &ev, 1).await;
    let two = slot_matches(&pool, &ev, 2).await;
    let three = slot_matches(&pool, &ev, 3).await;
    assert_eq!((one.len(), two.len(), three.len()), (2, 1, 2));
    let court_2_first = current_of(&pool, &courts[1]).await.unwrap();
    let token = |id: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT scorer_token FROM matches WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    for m in &one {
        type_result(&s, m, 5, 3, None).await;
    }
    clock(&s, &ev, ClockAction::End, 1, T + 1_000)
        .await
        .unwrap();
    let court_2_token = token(court_2_first.clone()).await;

    clock(&s, &ev, ClockAction::Start, 2, T + 2_000)
        .await
        .unwrap();
    assert_eq!(current_of(&pool, &courts[0]).await, Some(two[0].clone()));
    // Court 2 has no match in slot 2: it keeps its finished one, and its token.
    assert_eq!(
        current_of(&pool, &courts[1]).await,
        Some(court_2_first.clone())
    );
    assert_eq!(token(court_2_first.clone()).await, court_2_token);
    let kd = kind_data(
        &pool,
        &ev,
        Audience::Court {
            court_id: courts[1].clone(),
        },
    )
    .await;
    assert_eq!(kd["clock"]["slot"], 1);

    type_result(&s, &two[0], 4, 2, None).await;
    clock(&s, &ev, ClockAction::End, 2, T + 3_000)
        .await
        .unwrap();
    clock(&s, &ev, ClockAction::Start, 3, T + 4_000)
        .await
        .unwrap();
    let mut current = vec![
        current_of(&pool, &courts[0]).await.unwrap(),
        current_of(&pool, &courts[1]).await.unwrap(),
    ];
    current.sort();
    let mut expected = three.clone();
    expected.sort();
    assert_eq!(current, expected);
}

#[tokio::test]
async fn end_marks_done_and_flags() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    assert_eq!(flags(&pool, &ev, 1).await, vec![0, 0]);
    clock(&s, &ev, ClockAction::End, 1, T + 30_000)
        .await
        .unwrap();
    let row = slot_row(&pool, &ev, 1).await;
    assert_eq!(row.state, "done");
    assert_eq!(row.done_at_ms, Some(T + 30_000));
    assert_eq!(row.remaining_ms, 0);
    assert_eq!((row.starts_at_ms, row.ends_at_ms), (None, None));
    assert_eq!(flags(&pool, &ev, 1).await, vec![1, 1]);
    assert_eq!(flags(&pool, &ev, 2).await, vec![0, 0]);
    conflict(
        clock(&s, &ev, ClockAction::End, 1, T + 31_000).await,
        "clock_not_running",
    );
}

#[tokio::test]
async fn end_while_paused_marks_done() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 8, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    clock(&s, &ev, ClockAction::Pause, 1, T + 1_000)
        .await
        .unwrap();
    clock(&s, &ev, ClockAction::End, 1, T + 2_000)
        .await
        .unwrap();
    assert_eq!(slot_row(&pool, &ev, 1).await.state, "done");
    assert_eq!(flags(&pool, &ev, 1).await, vec![1, 1]);
}

#[tokio::test]
async fn play_on_a_done_slot_flags_time_up() {
    // A match of a done slot that loses its result gets flagged again by `on_result`.
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 4, 2).await;
    let one = slot_matches(&pool, &ev, 1).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();
    clock(&s, &ev, ClockAction::End, 1, T + 1_000)
        .await
        .unwrap();
    sqlx::query("UPDATE non_stop_matches SET needs_time_up = 0 WHERE event_id = ?")
        .bind(&ev.id)
        .execute(&pool)
        .await
        .unwrap();
    // A point on an unfinished match of a done slot flags it.
    send(&s, &one[0], "p1", vec![Command::Point { team: Team::A }]).await;
    let flagged: i64 =
        sqlx::query_scalar("SELECT needs_time_up FROM non_stop_matches WHERE match_id = ?")
            .bind(&one[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(flagged, 1);
    // A point on a match of a slot still ready flags nothing.
    let two = slot_matches(&pool, &ev, 2).await;
    send(&s, &two[0], "p2", vec![Command::Point { team: Team::A }]).await;
    assert_eq!(flags(&pool, &ev, 2).await, vec![0, 0]);
}

#[tokio::test]
async fn court_kind_data_follows_its_match() {
    let (s, pool) = setup().await;
    let (ev, _, courts) = started(&s, timed(), 4, 2).await;
    let court = Audience::Court {
        court_id: courts[0].clone(),
    };
    let first = current_of(&pool, &courts[0]).await.unwrap();
    let kd = kind_data(&pool, &ev, court.clone()).await;
    assert_eq!(kd["clock"]["slot"], 1);
    assert_eq!(kd["clock"]["state"], "ready");
    assert_eq!(kd["rounds"], 3);
    assert_eq!(kd["waves"], 1);
    assert!(kd.get("standings").is_none());
    assert!(kd.get("slots").is_none());
    // Still slot 1 while its finished match is current.
    type_result(&s, &first, 5, 3, None).await;
    assert_eq!(
        current_of(&pool, &courts[0]).await.as_deref(),
        Some(first.as_str())
    );
    assert_eq!(
        kind_data(&pool, &ev, court.clone()).await["clock"]["slot"],
        1
    );
    // No current match: its next match's slot.
    sqlx::query("UPDATE courts SET current_match_id = NULL WHERE id = ?")
        .bind(&courts[0])
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(kind_data(&pool, &ev, court).await["clock"]["slot"], 2);
    // An unknown court has no clock.
    let kd = kind_data(
        &pool,
        &ev,
        Audience::Court {
            court_id: "nope".into(),
        },
    )
    .await;
    assert_eq!(kd["clock"], Value::Null);
}

#[tokio::test]
async fn standings_from_results() {
    let (s, pool) = setup().await;
    let (ev, teams, _) = started(&s, timed_draws(), 4, 2).await;
    let ms = matches_of(&pool, &ev).await;
    type_result(&s, &ms[0].match_id, 5, 3, None).await;
    type_result(&s, &ms[1].match_id, 4, 4, None).await;
    type_result(&s, &ms[2].match_id, 3, 6, None).await;
    let outs = outcomes(&pool, &ev).await;
    assert_eq!(outs.len(), 3);
    assert_eq!(outs[1].winner, None);
    let expected = serde_json::to_value(standings(&teams, &outs)).unwrap();
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    assert_eq!(kd["standings"], expected);
    let rows = kd["standings"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().any(|r| r["drawn"] == 1));

    // The spectator sees the same, with no token anywhere.
    let view = padel_score_server::competitions::view::event_view(&s, &ev.spectator_token)
        .await
        .unwrap()
        .unwrap();
    let spectator = view.kind_data.unwrap();
    assert_eq!(spectator["standings"], expected);
    assert_eq!(spectator, kd);
    let text = spectator.to_string();
    let tokens: Vec<String> = sqlx::query_scalar(
        "SELECT scorer_token FROM matches WHERE id IN \
         (SELECT match_id FROM competition_matches WHERE event_id = ?1) \
         UNION ALL SELECT scorer_token FROM courts WHERE event_id = ?1 \
         UNION ALL SELECT spectator_token FROM courts WHERE event_id = ?1",
    )
    .bind(&ev.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(tokens.len() >= 8);
    for t in tokens.iter().chain(std::iter::once(&ev.spectator_token)) {
        assert!(
            !text.contains(t.as_str()),
            "token {t} in spectator kind_data"
        );
    }
}

#[tokio::test]
async fn typed_and_tapped_overtime_agree() {
    let (s, pool) = setup().await;
    let (ev, teams, _) = started(&s, timed(), 4, 2).await;
    let one = slot_matches(&pool, &ev, 1).await;
    // Tapped to 5-5, time up, then the overtime game to A.
    let mut level = Vec::new();
    for _ in 0..5 {
        level.extend([Team::A, Team::B]);
    }
    send(&s, &one[0], "tap", games(&level)).await;
    send(&s, &one[0], "time", vec![Command::TimeUp]).await;
    send(&s, &one[0], "ot", games(&[Team::A])).await;
    type_result(&s, &one[1], 5, 5, Some(Team::A)).await;

    let ms = matches_of(&pool, &ev).await;
    let kd = kind_data(&pool, &ev, Audience::Coordinator).await;
    let rows = kd["standings"].as_array().unwrap();
    let row = |id: &str| {
        let mut r = rows.iter().find(|r| r["team_id"] == id).unwrap().clone();
        r.as_object_mut().unwrap().remove("team_id");
        r
    };
    let by_id = |id: &str| ms.iter().find(|m| m.match_id == id).unwrap();
    let (tapped, typed) = (by_id(&one[0]), by_id(&one[1]));
    assert_eq!(row(&tapped.team_a_id), row(&typed.team_a_id));
    assert_eq!(row(&tapped.team_b_id), row(&typed.team_b_id));
    let winner = row(&tapped.team_a_id);
    assert_eq!(winner["won"], 1);
    assert_eq!(winner["points"], 3);
    assert_eq!(winner["games_won"], 5);
    assert_eq!(row(&tapped.team_b_id)["games_won"], 5);
    assert_eq!(teams.len(), 4);
}

async fn placings(pool: &SqlitePool, ev: &Competition) -> HashMap<String, (i64, i64)> {
    let rows = sqlx::query("SELECT player_id, place, place_to FROM placings WHERE event_id = ?")
        .bind(&ev.id)
        .fetch_all(pool)
        .await
        .unwrap();
    rows.iter()
        .map(|r| (r.get("player_id"), (r.get("place"), r.get("place_to"))))
        .collect()
}

/// The players of each team, by team id.
async fn players_of(pool: &SqlitePool, ev: &Competition) -> HashMap<String, [String; 2]> {
    let rows =
        sqlx::query("SELECT id, player1_id, player2_id FROM competition_teams WHERE event_id = ?")
            .bind(&ev.id)
            .fetch_all(pool)
            .await
            .unwrap();
    rows.iter()
        .map(|r| (r.get("id"), [r.get("player1_id"), r.get("player2_id")]))
        .collect()
}

#[tokio::test]
async fn finish_writes_computed_places() {
    let (s, pool) = setup().await;
    let (ev, teams, _) = started(&s, timed(), 4, 2).await;
    let ms = matches_of(&pool, &ev).await;
    type_result(&s, &ms[0].match_id, 5, 3, None).await;
    type_result(&s, &ms[1].match_id, 2, 6, None).await;
    type_result(&s, &ms[2].match_id, 4, 3, None).await;
    let expected = places_of(&standings(&teams, &outcomes(&pool, &ev).await));
    s.finish_competition(A, &ev.id, None, NOW + 5)
        .await
        .unwrap();
    let got = placings(&pool, &ev).await;
    assert_eq!(got.len(), 8);
    let players = players_of(&pool, &ev).await;
    for p in &expected {
        for player in &players[&p.team_id] {
            assert_eq!(
                got[player],
                (i64::from(p.place), i64::from(p.place_to)),
                "{}",
                p.team_id
            );
        }
    }
}

#[tokio::test]
async fn correction_after_finish_rewrites_placings() {
    let (s, pool) = setup().await;
    let (ev, _, _) = started(&s, timed(), 4, 2).await;
    let ms = matches_of(&pool, &ev).await;
    let m = &ms[0];
    type_result(&s, &m.match_id, 5, 3, None).await;
    s.finish_competition(A, &ev.id, None, NOW + 5)
        .await
        .unwrap();
    let players = players_of(&pool, &ev).await;
    let place_of = |got: &HashMap<String, (i64, i64)>, team: &str| got[&players[team][0]];
    let before = placings(&pool, &ev).await;
    assert_eq!(place_of(&before, &m.team_a_id), (1, 1));
    assert_eq!(place_of(&before, &m.team_b_id), (2, 2));
    type_result(&s, &m.match_id, 3, 5, None).await;
    let after = placings(&pool, &ev).await;
    assert_eq!(after.len(), 8);
    assert_eq!(place_of(&after, &m.team_a_id), (2, 2));
    assert_eq!(place_of(&after, &m.team_b_id), (1, 1));
}

#[tokio::test]
async fn tournament_clock_is_not_non_stop() {
    let (s, _) = setup().await;
    let ev = s
        .create_competition(
            A,
            NewCompetition {
                name: "Cup".into(),
                kind: Kind::Tournament,
                format: Format::Sets(SetRules {
                    deuce: DeuceRule::GoldenPoint,
                    sets: SetsFormat::BestOf3,
                    deciding: DecidingSet::SuperTieBreak,
                }),
                settings: None,
            },
            10,
        )
        .await
        .unwrap();
    conflict(
        clock(&s, &ev, ClockAction::Start, 1, T).await,
        "not_non_stop",
    );
}

#[test]
fn clock_action_is_snake_case() {
    let a: ClockAction = serde_json::from_value(json!("resume")).unwrap();
    assert!(matches!(a, ClockAction::Resume));
    assert!(serde_json::from_value::<ClockAction>(json!("Resume")).is_err());
}

#[tokio::test]
async fn runner_helpers_find_due_work() {
    let (s, pool) = setup().await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(!any_running_non_stop_in(&mut conn).await.unwrap());
    assert_eq!(next_deadline_in(&mut conn).await.unwrap(), None);
    drop(conn);
    let (ev, _, _) = started(&s, timed(), 4, 2).await;
    clock(&s, &ev, ClockAction::Start, 1, T).await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    assert!(any_running_non_stop_in(&mut conn).await.unwrap());
    // Not gone yet: the next deadline is the go.
    assert_eq!(next_deadline_in(&mut conn).await.unwrap(), Some(T + 5_000));
    assert!(due_go_in(&mut conn, T + 4_999).await.unwrap().is_empty());
    assert_eq!(
        due_go_in(&mut conn, T + 5_000).await.unwrap(),
        vec![(ev.id.clone(), 1)]
    );
    drop(conn);
    let mut tx = pool.begin().await.unwrap();
    went_in(&mut tx, &ev.id, 1, T + 5_000).await.unwrap();
    tx.commit().await.unwrap();

    let mut conn = pool.acquire().await.unwrap();
    assert!(due_go_in(&mut conn, T + 5_000).await.unwrap().is_empty());
    assert_eq!(
        next_deadline_in(&mut conn).await.unwrap(),
        Some(T + 905_000)
    );
    assert!(
        due_slots_in(&mut conn, T + 904_999)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        due_slots_in(&mut conn, T + 905_000).await.unwrap(),
        vec![(ev.id.clone(), 1)]
    );
    assert!(pending_time_up_in(&mut conn).await.unwrap().is_empty());
    drop(conn);
    let mut tx = pool.begin().await.unwrap();
    mark_done_in(&mut tx, &ev.id, 1, T + 905_000).await.unwrap();
    tx.commit().await.unwrap();

    let mut one = slot_matches(&pool, &ev, 1).await;
    one.sort();
    let mut conn = pool.acquire().await.unwrap();
    assert!(
        due_slots_in(&mut conn, T + 905_000)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(next_deadline_in(&mut conn).await.unwrap(), None);
    let mut pending = pending_time_up_in(&mut conn).await.unwrap();
    pending.sort();
    assert_eq!(pending, one);
    clear_time_up_in(&mut conn, &one[0]).await.unwrap();
    assert_eq!(
        pending_time_up_in(&mut conn).await.unwrap(),
        vec![one[1].clone()]
    );
    drop(conn);

    // A slot ended before its go is due for the go (its practice points are cleared first),
    // and its matches are not pending a time-up until it has gone.
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("UPDATE non_stop_slots SET started_at_ms = NULL WHERE event_id = ? AND slot = 1")
        .bind(&ev.id)
        .execute(&mut *conn)
        .await
        .unwrap();
    assert_eq!(
        due_go_in(&mut conn, T).await.unwrap(),
        vec![(ev.id.clone(), 1)]
    );
    assert!(pending_time_up_in(&mut conn).await.unwrap().is_empty());
    drop(conn);
    let mut tx = pool.begin().await.unwrap();
    went_in(&mut tx, &ev.id, 1, T + 1).await.unwrap();
    tx.commit().await.unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        pending_time_up_in(&mut conn).await.unwrap(),
        vec![one[1].clone()]
    );
    drop(conn);

    // A finished event has nothing due.
    s.finish_competition(A, &ev.id, None, NOW + 5)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    assert!(due_go_in(&mut conn, T).await.unwrap().is_empty());
    assert!(pending_time_up_in(&mut conn).await.unwrap().is_empty());
    assert!(!any_running_non_stop_in(&mut conn).await.unwrap());
}
