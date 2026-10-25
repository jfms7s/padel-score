#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::sync::Arc;

use padel_score_server::retention::{ANON_RETENTION_SECS, purge_inactive, purge_once};
use padel_score_server::store::{EventState, NewMatch, Store, Teams, append};
use padel_score_server::{AppState, Config, db};
use padel_scoring::{
    DecidingSet, DeuceRule, Event, Format, Player, SetRules, SetsFormat, Start, Team,
};
use sqlx::SqlitePool;

const DAY: i64 = 24 * 3600;
const NOW: i64 = 100 * DAY;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn new_match() -> NewMatch {
    NewMatch {
        format: Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::BestOf3,
            deciding: DecidingSet::FullSet,
        }),
        start: Start {
            first_server: Player {
                team: Team::A,
                index: 0,
            },
            other_first_server: 0,
        },
        teams: Teams {
            a: ["Ana".into(), "Bea".into()],
            b: ["Cris".into(), "Dani".into()],
        },
        owner_user_id: None,
    }
}

/// A match whose last activity was `idle` seconds before `NOW`, with one event and one result.
async fn idle_match(store: &Store, idle: i64) -> padel_score_server::store::StoredMatch {
    let m = store.create_match(new_match(), NOW - idle).await.unwrap();
    let mut tx = store.begin().await.unwrap();
    append(&mut tx, &m.id, 1, Event::Point(Team::A), "c1", NOW - idle)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    m
}

async fn exists(store: &Store, m: &padel_score_server::store::StoredMatch) -> bool {
    store
        .by_scorer_token(&m.scorer_token)
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn purges_only_old_anonymous_matches() {
    let (store, _) = setup().await;
    let old = idle_match(&store, 31 * DAY).await;
    let recent = idle_match(&store, 29 * DAY).await;
    let purged = purge_inactive(&store, NOW).await.unwrap();
    assert_eq!(purged, vec![old.id.clone()]);
    assert!(!exists(&store, &old).await);
    assert!(exists(&store, &recent).await);
}

#[tokio::test]
async fn exactly_thirty_days_idle_is_kept() {
    let (store, _) = setup().await;
    let edge = idle_match(&store, ANON_RETENTION_SECS).await;
    let past = idle_match(&store, ANON_RETENTION_SECS + 1).await;
    let purged = purge_inactive(&store, NOW).await.unwrap();
    assert_eq!(purged, vec![past.id.clone()]);
    assert!(exists(&store, &edge).await);
}

#[tokio::test]
async fn owned_matches_are_never_purged() {
    let (store, pool) = setup().await;
    let owned = idle_match(&store, 90 * DAY).await;
    sqlx::query("UPDATE matches SET owner_user_id = 'u1' WHERE id = ?")
        .bind(&owned.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(purge_inactive(&store, NOW).await.unwrap().is_empty());
    assert!(exists(&store, &owned).await);
}

#[tokio::test]
async fn purge_removes_events_too() {
    let (store, pool) = setup().await;
    let old = idle_match(&store, 31 * DAY).await;
    let kept = idle_match(&store, DAY).await;
    sqlx::query("INSERT INTO command_results (match_id, client_id, status, body) VALUES (?, 'c1', 200, '{}')")
        .bind(&old.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(store.events(&old.id).await.unwrap().len(), 1);
    assert_eq!(purge_inactive(&store, NOW).await.unwrap().len(), 1);
    assert!(store.events(&old.id).await.unwrap().is_empty());
    let (results,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM command_results")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(results, 0);
    let events = store.events(&kept.id).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].state, EventState::Effective);
}

#[tokio::test]
async fn purge_once_closes_hub_entries_of_purged_matches() {
    let (store, _) = setup().await;
    let old = idle_match(&store, 31 * DAY).await;
    let recent = idle_match(&store, DAY).await;
    let state = AppState::new(Arc::new(Config::from_map(&HashMap::new()).unwrap()), store);
    let _old_sub = state.hub.subscribe(&old.id);
    let _recent_sub = state.hub.subscribe(&recent.id);
    assert_eq!(purge_once(&state, NOW).await, 1);
    assert!(!state.hub.is_watched(&old.id));
    assert!(state.hub.is_watched(&recent.id));
}

#[tokio::test]
async fn run_purges_at_startup_and_stops_on_shutdown() {
    let (store, _) = setup().await;
    // Real time: a match idle since the epoch is far past retention.
    let old = store.create_match(new_match(), 1).await.unwrap();
    let state = AppState::new(
        Arc::new(Config::from_map(&HashMap::new()).unwrap()),
        store.clone(),
    );
    let task = tokio::spawn(padel_score_server::retention::run(state.clone()));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while exists(&store, &old).await {
        assert!(tokio::time::Instant::now() < deadline, "never purged");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    state.shutdown.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("run did not stop")
        .unwrap();
}
