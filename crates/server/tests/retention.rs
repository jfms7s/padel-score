#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

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

#[tokio::test]
async fn retention_purges_expired_auth_but_keeps_owned_matches() {
    let (state, pool, _, admin_id, _) = support::with_admin().await;
    let mut owned_new = new_match();
    owned_new.owner_user_id = Some(admin_id.clone());
    let owned = state
        .store
        .create_match(owned_new, NOW - 90 * DAY)
        .await
        .unwrap();
    let anon = idle_match(&state.store, 31 * DAY).await;
    // One expired and one live row of each kind (the admin's own session expires far later).
    for (name, expires) in [("old", NOW - 1), ("live", NOW + 1)] {
        let sqls = [
            "INSERT INTO sessions (token_hash, user_id, expires_at, created_at) VALUES (?1, ?2, ?3, 0)",
            "INSERT INTO invites (id, code_hash, created_by, created_at, expires_at) VALUES (?1, ?1, ?2, 0, ?3)",
            "INSERT INTO device_codes (id, user_code, device_code_hash, device_name, created_at, expires_at) VALUES (?1, ?1, ?1, 'W', 0, ?3)",
            "INSERT INTO password_resets (code_hash, user_id, created_by, created_at, expires_at) VALUES (?1, ?2, ?2, 0, ?3)",
        ];
        for sql in sqls {
            sqlx::query(sql)
                .bind(name)
                .bind(&admin_id)
                .bind(expires)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    assert_eq!(purge_once(&state, NOW).await, 1);
    assert!(!exists(&state.store, &anon).await);
    assert!(exists(&state.store, &owned).await);
    let counts = [
        (
            "sessions",
            "SELECT COUNT(*) FROM sessions WHERE expires_at = ?",
        ),
        (
            "invites",
            "SELECT COUNT(*) FROM invites WHERE expires_at = ?",
        ),
        (
            "device_codes",
            "SELECT COUNT(*) FROM device_codes WHERE expires_at = ?",
        ),
        (
            "password_resets",
            "SELECT COUNT(*) FROM password_resets WHERE expires_at = ?",
        ),
    ];
    for (table, sql) in counts {
        let (old,): (i64,) = sqlx::query_as(sql)
            .bind(NOW - 1)
            .fetch_one(&pool)
            .await
            .unwrap();
        let (live,): (i64,) = sqlx::query_as(sql)
            .bind(NOW + 1)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((table, old, live), (table, 0, 1));
    }
}
