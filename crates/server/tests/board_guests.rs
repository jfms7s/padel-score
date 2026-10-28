#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{BoardChange, BoardView, MAX_BOARD_MEMBERS, NewBoard, WEEK_SECS};
use padel_score_server::db;
use padel_score_server::players::PlayerSlots;
use padel_score_server::starting::{StartedFrom, create_match_from};
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use sqlx::SqlitePool;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
/// The test board's week starts a day after `T0`.
const START: i64 = T0 + DAY;
const HOUR: i64 = 3600;
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

/// Everyone adds `[start, start + 1h)`, a second apart in the order given.
async fn add_free_for(s: &Store, token: &str, who: &[&str], start: i64) {
    for (i, id) in who.iter().enumerate() {
        let now = NOW + i64::try_from(i).unwrap();
        s.add_free(token, id, start, start + HOUR, now)
            .await
            .unwrap();
    }
}

/// A guest's id and key.
async fn guest(s: &Store, token: &str, name: &str) -> (String, String) {
    let j = s.join_as_guest(token, name, NOW).await.unwrap();
    (j.view.me.unwrap().user_id, j.guest_key)
}

async fn view(s: &Store, token: &str, who: &str, now: i64) -> BoardView {
    s.board_view(token, Some(who), now).await.unwrap().unwrap()
}

async fn creator_of(s: &Store, token: &str) -> String {
    view(s, token, "nobody", NOW).await.board.creator_user_id
}

fn player_ids(v: &BoardView) -> Vec<String> {
    v.games[0]
        .players
        .iter()
        .map(|p| p.user_id.clone())
        .collect()
}

#[tokio::test]
async fn a_guest_leaving_ends_the_guest() {
    let (s, _, token) = open_board().await;
    let (id, key) = guest(&s, &token, "Zé").await;
    s.add_free(&token, &id, START, START + HOUR, NOW)
        .await
        .unwrap();
    s.leave_board(&token, &id, NOW + 1).await.unwrap();
    let v = view(&s, &token, "nobody", NOW + 1).await;
    assert!(v.members.iter().all(|m| m.user_id != id));
    assert!(v.pieces.iter().all(|p| p.user_id != id));
    assert_eq!(s.guest_for_key(&token, &key).await.unwrap(), None);
    s.join_as_guest(&token, "zé", NOW + 2).await.unwrap();
}

#[tokio::test]
async fn creator_removes_a_guest_and_the_line_fills() {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let c = user(&s, "Marta").await;
    let e = user(&s, "Joana").await;
    let token = board(&s, &a, true).await;
    for u in [&b, &c, &e] {
        s.join_board(&token, u, NOW).await.unwrap();
    }
    let (g, _) = guest(&s, &token, "Zé").await;
    add_free_for(&s, &token, &[&a, &b, &c, &g, &e], START).await;
    let before = view(&s, &token, &a, NOW + 10).await;
    assert_eq!(before.games.len(), 1);
    assert!(player_ids(&before).contains(&g));
    assert_eq!(before.games[0].line, vec![e.clone()]);

    let after = s.remove_guest(&token, &a, &g, NOW + 20).await.unwrap();
    assert!(!player_ids(&after).contains(&g));
    assert!(player_ids(&after).contains(&e));
    assert!(after.games[0].line.is_empty());
    assert!(after.members.iter().all(|m| m.user_id != g));
    assert!(after.pieces.iter().all(|p| p.user_id != g));
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_ranges WHERE user_id = ?")
        .bind(&g)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn only_the_creator_removes() {
    let (s, _, token) = open_board().await;
    let creator = creator_of(&s, &token).await;
    let b = user(&s, "Rui").await;
    s.join_board(&token, &b, NOW).await.unwrap();
    let (g, key) = guest(&s, &token, "Zé").await;
    conflict(s.remove_guest(&token, &b, &g, NOW).await, "creator_only");
    conflict(s.remove_guest(&token, &g, &g, NOW).await, "creator_only");
    assert!(s.guest_for_key(&token, &key).await.unwrap().is_some());
    // A user id, an unknown id and a guest of another board are not this board's guests.
    assert!(matches!(
        s.remove_guest(&token, &creator, &b, NOW).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.remove_guest(&token, &creator, "guest_nope", NOW).await,
        Err(StoreError::NotFound)
    ));
    let c = user(&s, "Marta").await;
    let other = board(&s, &c, true).await;
    assert!(matches!(
        s.remove_guest(&other, &c, &g, NOW).await,
        Err(StoreError::NotFound)
    ));
    assert!(s.guest_for_key(&token, &key).await.unwrap().is_some());
}

fn guests_off() -> BoardChange {
    BoardChange {
        allow_guests: Some(false),
        ..BoardChange::default()
    }
}

#[tokio::test]
async fn removing_works_with_guests_off() {
    let (s, _, token) = open_board().await;
    let creator = creator_of(&s, &token).await;
    let (g, key) = guest(&s, &token, "Zé").await;
    s.update_board(&token, &creator, guests_off(), NOW)
        .await
        .unwrap();
    let v = s.remove_guest(&token, &creator, &g, NOW + 1).await.unwrap();
    assert!(v.members.iter().all(|m| m.user_id != g));
    assert_eq!(s.guest_for_key(&token, &key).await.unwrap(), None);
}

fn new_match(owner: &str) -> NewMatch {
    NewMatch {
        format: Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::One,
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
            a: ["x".into(), "y".into()],
            b: ["z".into(), "w".into()],
        },
        owner_user_id: Some(owner.to_owned()),
        players: PlayerSlots::default(),
    }
}

#[tokio::test]
async fn removing_a_guest_keeps_their_history_seats() {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let c = user(&s, "Marta").await;
    let token = board(&s, &a, true).await;
    for u in [&b, &c] {
        s.join_board(&token, u, NOW).await.unwrap();
    }
    let (g, _) = guest(&s, &token, "Zé").await;
    let people = [a.as_str(), b.as_str(), c.as_str(), g.as_str()];
    // One game that will have ended, and one that is being played.
    let played_start = START + 2 * HOUR;
    add_free_for(&s, &token, &people, START).await;
    add_free_for(&s, &token, &people, played_start).await;
    let v = view(&s, &token, &a, NOW + 10).await;
    assert_eq!(v.games.len(), 2);
    let played = v.games.iter().find(|g| g.start == played_start).unwrap();
    let from = StartedFrom::Game {
        id: played.id.clone(),
    };
    create_match_from(&s, new_match(&a), &from, played_start)
        .await
        .unwrap();

    let now = played_start + 600;
    let after = s.remove_guest(&token, &a, &g, now).await.unwrap();
    assert_eq!(after.games.len(), 2);
    for game in &after.games {
        assert!(
            game.players.iter().any(|p| p.user_id == g && p.guest),
            "guest seat missing in the game at {}",
            game.start
        );
        assert!(game.players.iter().any(|p| p.name == "Zé"));
    }
    let queued: Vec<String> = sqlx::query_scalar("SELECT payload FROM push_outbox")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        queued.iter().all(|p| !p.contains("player_left")),
        "unexpected player_left: {queued:?}"
    );
    let gone: Option<i64> = sqlx::query_scalar("SELECT gone_at FROM board_guests WHERE id = ?")
        .bind(&g)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(gone, Some(now));
}

#[tokio::test]
async fn turning_guests_off_keeps_existing_guests() {
    let (s, _, token) = open_board().await;
    let creator = creator_of(&s, &token).await;
    let (g, key) = guest(&s, &token, "Zé").await;
    s.update_board(&token, &creator, guests_off(), NOW + 1)
        .await
        .unwrap();
    assert_eq!(
        s.guest_for_key(&token, &key).await.unwrap(),
        Some(g.clone())
    );
    s.add_free(&token, &g, START, START + HOUR, NOW + 2)
        .await
        .unwrap();
    conflict(s.join_as_guest(&token, "Rui", NOW + 3).await, "guests_off");
    conflict(s.pick_guest(&token, &g, NOW + 3).await, "guests_off");
}
