#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::{HashMap, HashSet};

use padel_score_server::competitions::courts::{EventMatchRow, event_matches_in};
use padel_score_server::competitions::{Competition, Kind, NewCompetition};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DeuceRule, Format};
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

async fn create(s: &Store, settings: Option<Value>) -> Result<Competition, StoreError> {
    s.create_competition(
        A,
        NewCompetition {
            name: "Americano".into(),
            kind: Kind::NonStop,
            format: timed(),
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
    let ev = create(s, settings).await.unwrap();
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
