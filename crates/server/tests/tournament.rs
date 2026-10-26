#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::{HashMap, HashSet};

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, apply_command,
};
use padel_score_server::competitions::courts::{Court, courts_in, event_matches_in};
use padel_score_server::competitions::tournament::store::fill_courts_in;
use padel_score_server::competitions::{Competition, Kind, NewCompetition, Status};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat, Team};
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

async fn finish(s: &Store, match_id: &str) {
    let (m, _) = s.snapshot(match_id).await.unwrap().unwrap();
    let req = CommandRequest {
        client_id: "finish".into(),
        batch: false,
        base_revision: 0,
        commands: vec![Command::Point { team: Team::A }; 24],
    };
    match apply_command(s, &m, req, NOW, Authority::Scorer)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(_) => {}
        other => panic!("expected Applied, got {other:?}"),
    }
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
    finish(&s, &q1).await;
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
    let (ev, _) = event(&s, full(), 8, 2).await;
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
    finish(&s, &q1).await;
    fill(&s, &ev).await;
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
