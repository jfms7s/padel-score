#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{MAX_BOARD_MEMBERS, NewBoard, WEEK_SECS};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use sqlx::SqlitePool;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
/// The test board's week starts a day after `T0`.
const START: i64 = T0 + DAY;
const NOW: i64 = T0 + 10;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

async fn user(s: &Store, name: &str) -> String {
    s.create_user(
        NewUser {
            email: format!("{}@x.test", name.to_lowercase()),
            display_name: name.to_owned(),
            password_hash: "x".to_owned(),
        },
        1,
    )
    .await
    .unwrap()
    .id
}

fn new_board(starts_at: i64, allow_guests: bool) -> NewBoard {
    NewBoard {
        allow_guests,
        title: "Thursday padel".to_owned(),
        place: Some("Club".to_owned()),
        time_zone: "Europe/Lisbon".to_owned(),
        starts_at,
        ends_at: starts_at + WEEK_SECS,
    }
}

/// The token of a board created by `creator` at `T0`.
async fn board(s: &Store, creator: &str, allow_guests: bool) -> String {
    s.create_board(creator, new_board(START, allow_guests), T0)
        .await
        .unwrap()
        .board
        .token
}

async fn open_board() -> (Store, SqlitePool, String) {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a, true).await;
    (s, pool, token)
}

fn conflict(r: Result<impl std::fmt::Debug, StoreError>, code: &str) {
    match r {
        Err(StoreError::Conflict(c)) => assert_eq!(c, code),
        other => panic!("expected Conflict({code}), got {other:?}"),
    }
}

#[tokio::test]
async fn guests_off_refuses_join_and_pick() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a, false).await;
    conflict(s.join_as_guest(&token, "Zé", NOW).await, "guests_off");
    conflict(s.pick_guest(&token, "guest_x", NOW).await, "guests_off");
}

#[tokio::test]
async fn join_as_guest_makes_a_member_with_a_key() {
    let (s, _, token) = open_board().await;
    let j = s.join_as_guest(&token, "  Zé ", NOW).await.unwrap();
    let me = j.view.me.clone().unwrap();
    assert!(me.user_id.starts_with("guest_"));
    assert!(me.member);
    assert!(!me.creator);
    assert!(me.guest);
    let m = j.view.members.iter().find(|m| m.name == "Zé").unwrap();
    assert!(m.guest);
    assert_eq!(m.user_id, me.user_id);
    assert_eq!(
        s.guest_for_key(&token, &j.guest_key).await.unwrap(),
        Some(me.user_id)
    );
}

#[tokio::test]
async fn guest_names_are_unique_per_board_ignoring_case() {
    let (s, _, token) = open_board().await;
    s.join_as_guest(&token, "Zé", NOW).await.unwrap();
    conflict(s.join_as_guest(&token, "zé", NOW).await, "guest_name_taken");
    let b = user(&s, "Rui").await;
    let other = board(&s, &b, true).await;
    s.join_as_guest(&other, "Zé", NOW).await.unwrap();
}

#[tokio::test]
async fn guest_name_rules() {
    let (s, _, token) = open_board().await;
    for name in ["  ".to_owned(), "x".repeat(41)] {
        assert!(matches!(
            s.join_as_guest(&token, &name, NOW).await,
            Err(StoreError::InvalidField("name"))
        ));
    }
}

#[tokio::test]
async fn guests_count_towards_the_member_cap() {
    let (s, _, token) = open_board().await;
    for i in 1..MAX_BOARD_MEMBERS {
        let u = user(&s, &format!("User{i}")).await;
        s.join_board(&token, &u, NOW).await.unwrap();
    }
    conflict(s.join_as_guest(&token, "Zé", NOW).await, "board_full");
}

#[tokio::test]
async fn pick_gives_a_second_key_and_both_work() {
    let (s, _, token) = open_board().await;
    let first = s.join_as_guest(&token, "Zé", NOW).await.unwrap();
    let id = first.view.me.clone().unwrap().user_id;
    let second = s.pick_guest(&token, &id, NOW + 1).await.unwrap();
    assert_ne!(first.guest_key, second.guest_key);
    assert_eq!(second.view.me.clone().unwrap().user_id, id);
    for key in [&first.guest_key, &second.guest_key] {
        assert_eq!(
            s.guest_for_key(&token, key).await.unwrap(),
            Some(id.clone())
        );
    }
}

#[tokio::test]
async fn pick_an_unknown_or_gone_guest_is_not_found() {
    let (s, pool, token) = open_board().await;
    assert!(matches!(
        s.pick_guest(&token, "guest_nope", NOW).await,
        Err(StoreError::NotFound)
    ));
    let j = s.join_as_guest(&token, "Zé", NOW).await.unwrap();
    let id = j.view.me.clone().unwrap().user_id;
    sqlx::query("UPDATE board_guests SET gone_at = ? WHERE id = ?")
        .bind(NOW)
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        s.pick_guest(&token, &id, NOW + 1).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(s.guest_for_key(&token, &j.guest_key).await.unwrap(), None);
}

#[tokio::test]
async fn key_is_board_scoped() {
    let (s, _, token) = open_board().await;
    let b = user(&s, "Rui").await;
    let other = board(&s, &b, true).await;
    let j = s.join_as_guest(&token, "Zé", NOW).await.unwrap();
    assert!(
        s.guest_for_key(&token, &j.guest_key)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(s.guest_for_key(&other, &j.guest_key).await.unwrap(), None);
    assert_eq!(s.guest_for_key(&token, "made-up").await.unwrap(), None);
}

#[tokio::test]
async fn over_board_refuses() {
    let (s, _, token) = open_board().await;
    let late = START + WEEK_SECS;
    conflict(s.join_as_guest(&token, "Zé", late).await, "board_over");
    conflict(s.pick_guest(&token, "guest_x", late).await, "board_over");
}
