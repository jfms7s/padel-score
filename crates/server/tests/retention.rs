#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::collections::HashMap;
use std::sync::Arc;

use padel_score_server::retention::{RETENTION_SECS, purge_inactive, purge_once};
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
        players: Default::default(),
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
    let edge = idle_match(&store, RETENTION_SECS).await;
    let past = idle_match(&store, RETENTION_SECS + 1).await;
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

async fn count(pool: &SqlitePool, sql: &'static str, id: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_result(pool: &SqlitePool, match_id: &str) {
    sqlx::query(
        "INSERT INTO command_results (match_id, client_id, status, body) VALUES (?, 'c1', 200, '{}')",
    )
    .bind(match_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn results_of(pool: &SqlitePool, match_id: &str) -> i64 {
    count(
        pool,
        "SELECT COUNT(*) FROM command_results WHERE match_id = ?",
        match_id,
    )
    .await
}

#[tokio::test]
async fn stale_command_results_are_purged_recent_kept() {
    let (state, pool, _, admin_id, _) = support::with_admin().await;
    let mut owned = Vec::new();
    for idle in [31 * DAY, RETENTION_SECS, DAY] {
        let m = idle_match(&state.store, idle).await;
        sqlx::query("UPDATE matches SET owner_user_id = ? WHERE id = ?")
            .bind(&admin_id)
            .bind(&m.id)
            .execute(&pool)
            .await
            .unwrap();
        add_result(&pool, &m.id).await;
        owned.push(m);
    }
    assert_eq!(purge_once(&state, NOW).await, 0);
    let [old, edge, recent] = owned.as_slice() else {
        unreachable!()
    };
    assert_eq!(results_of(&pool, &old.id).await, 0);
    assert_eq!(
        results_of(&pool, &edge.id).await,
        1,
        "exactly 30 days is kept"
    );
    assert_eq!(results_of(&pool, &recent.id).await, 1);
    for m in &owned {
        assert!(exists(&state.store, m).await);
        assert_eq!(state.store.events(&m.id).await.unwrap().len(), 1);
    }
}

/// A board that ended `ended` seconds before `NOW`, with a member, a range, a guest, a played
/// game (pointing at `match_id`, if any), its players, a line head and a request. Returns the board id.
async fn ended_board(
    pool: &SqlitePool,
    creator: &str,
    ended: i64,
    match_id: Option<&str>,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let game = format!("{id}-g1");
    let guest = format!("guest_{id}");
    let ends = NOW - ended;
    let starts = ends - 7 * DAY;
    let stmts: [(&str, Vec<String>); 9] = [
        (
            "INSERT INTO boards (id, token, creator_user_id, title, place, time_zone, starts_at, \
             ends_at, created_at, allow_guests) VALUES (?1, ?1, ?2, 'Week', NULL, 'UTC', ?3, ?4, ?3, 1)",
            vec![
                id.clone(),
                creator.into(),
                starts.to_string(),
                ends.to_string(),
            ],
        ),
        (
            "INSERT INTO board_members (board_id, user_id, joined_at) VALUES (?1, ?2, ?3)",
            vec![id.clone(), creator.into(), starts.to_string()],
        ),
        (
            "INSERT INTO board_ranges (id, board_id, user_id, starts_at, ends_at, created_at) \
             VALUES (?1 || '-r1', ?1, ?2, ?3, ?3 + 3600, ?3)",
            vec![id.clone(), creator.into(), starts.to_string()],
        ),
        (
            "INSERT INTO board_guests (id, board_id, name, name_key, created_at) \
             VALUES (?2, ?1, 'Gil', 'gil', ?3)",
            vec![id.clone(), guest.clone(), starts.to_string()],
        ),
        (
            "INSERT INTO board_guest_keys (key_hash, guest_id, created_at) VALUES (?1, ?2, ?3)",
            vec![format!("{id}-key"), guest.clone(), starts.to_string()],
        ),
        (
            "INSERT INTO board_games (id, board_id, starts_at, ends_at, formed_at, match_id, played_at) \
             VALUES (?1, ?2, ?3, ?3 + 5400, ?3, NULLIF(?4, ''), ?3)",
            vec![
                game.clone(),
                id.clone(),
                starts.to_string(),
                match_id.unwrap_or_default().to_owned(),
            ],
        ),
        (
            "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) VALUES (?1, ?2, 0, ?3)",
            vec![game.clone(), creator.into(), starts.to_string()],
        ),
        (
            "INSERT INTO board_line_heads (game_id, user_id, at) VALUES (?1, ?2, ?3)",
            vec![game.clone(), guest.clone(), starts.to_string()],
        ),
        (
            "INSERT INTO board_requests (id, board_id, kind, from_user_id, game_id, game_version, \
             to_user_id, created_at) VALUES (?1 || '-q1', ?1, 'swap', ?2, ?3, 0, ?4, ?5)",
            vec![
                id.clone(),
                creator.into(),
                game.clone(),
                guest.clone(),
                starts.to_string(),
            ],
        ),
    ];
    for (sql, binds) in stmts {
        let mut q = sqlx::query(sql);
        for b in binds {
            q = q.bind(b);
        }
        q.execute(pool).await.unwrap();
    }
    id
}

const BOARD_TABLES: [&str; 6] = [
    "SELECT COUNT(*) FROM boards WHERE id = ?",
    "SELECT COUNT(*) FROM board_members WHERE board_id = ?",
    "SELECT COUNT(*) FROM board_ranges WHERE board_id = ?",
    "SELECT COUNT(*) FROM board_guests WHERE board_id = ?",
    "SELECT COUNT(*) FROM board_games WHERE board_id = ?",
    "SELECT COUNT(*) FROM board_requests WHERE board_id = ?",
];

async fn board_rows(pool: &SqlitePool, id: &str) -> Vec<i64> {
    let mut out = Vec::new();
    for sql in BOARD_TABLES {
        out.push(count(pool, sql, id).await);
    }
    for sql in [
        "SELECT COUNT(*) FROM board_game_players WHERE game_id = ? || '-g1'",
        "SELECT COUNT(*) FROM board_line_heads WHERE game_id = ? || '-g1'",
        "SELECT COUNT(*) FROM board_guest_keys WHERE guest_id = 'guest_' || ?",
    ] {
        out.push(count(pool, sql, id).await);
    }
    out
}

#[tokio::test]
async fn boards_ended_long_ago_are_purged() {
    let (state, pool, _, admin_id, _) = support::with_admin().await;
    let old = ended_board(&pool, &admin_id, 31 * DAY, None).await;
    let edge = ended_board(&pool, &admin_id, RETENTION_SECS, None).await;
    let recent = ended_board(&pool, &admin_id, DAY, None).await;
    assert_eq!(board_rows(&pool, &old).await, vec![1; 9]);
    purge_once(&state, NOW).await;
    assert_eq!(board_rows(&pool, &old).await, vec![0; 9]);
    assert_eq!(
        board_rows(&pool, &edge).await,
        vec![1; 9],
        "exactly 30 days is kept"
    );
    assert_eq!(board_rows(&pool, &recent).await, vec![1; 9]);
    let broken: Vec<(String,)> = sqlx::query_as("SELECT \"table\" FROM pragma_foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
}

#[tokio::test]
async fn a_match_started_from_a_purged_board_survives() {
    let (state, pool, _, admin_id, _) = support::with_admin().await;
    let m = idle_match(&state.store, DAY).await;
    sqlx::query("UPDATE matches SET owner_user_id = ? WHERE id = ?")
        .bind(&admin_id)
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();
    let board = ended_board(&pool, &admin_id, 31 * DAY, Some(&m.id)).await;
    purge_once(&state, NOW).await;
    assert_eq!(board_rows(&pool, &board).await, vec![0; 9]);
    assert!(exists(&state.store, &m).await);
    assert_eq!(state.store.events(&m.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_non_stop_runners_time_up_results_are_kept() {
    let (state, pool, _, _, _) = support::with_admin().await;
    let m = idle_match(&state.store, 31 * DAY).await;
    sqlx::query("UPDATE matches SET owner_user_id = 'u1' WHERE id = ?")
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();
    add_result(&pool, &m.id).await;
    sqlx::query(
        "INSERT INTO command_results (match_id, client_id, status, body) \
         VALUES (?1, 'time_up:' || ?1 || ':1', 200, '{}')",
    )
    .bind(&m.id)
    .execute(&pool)
    .await
    .unwrap();
    purge_once(&state, NOW).await;
    let kept: Vec<(String,)> =
        sqlx::query_as("SELECT client_id FROM command_results WHERE match_id = ?")
            .bind(&m.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(kept, vec![(format!("time_up:{}:1", m.id),)]);
}

#[tokio::test]
async fn a_failed_purge_does_not_stop_the_others() {
    let (state, pool, _, admin_id, _) = support::with_admin().await;
    let anon = idle_match(&state.store, 31 * DAY).await;
    let board = ended_board(&pool, &admin_id, 31 * DAY, None).await;
    // The command results purge (between the two) now fails.
    sqlx::query("DROP TABLE command_results")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(purge_once(&state, NOW).await, 1);
    assert!(!exists(&state.store, &anon).await);
    assert_eq!(count(&pool, BOARD_TABLES[0], &board).await, 0);
}
