#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Game actions on a board: Leave, Give my place and Take the open place, through the store.

use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{BoardView, GameState, NewBoard, WEEK_SECS};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use serde_json::Value;
use sqlx::SqlitePool;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
const HOUR: i64 = 3600;
const HALF: i64 = 1800;
/// The test board's week starts a day after `T0`.
const START: i64 = T0 + DAY;

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

async fn board(s: &Store, creator: &str) -> String {
    s.create_board(
        creator,
        NewBoard {
            title: "Thursday padel".to_owned(),
            place: None,
            time_zone: "Europe/Lisbon".to_owned(),
            starts_at: START,
            ends_at: START + WEEK_SECS,
        },
        T0,
    )
    .await
    .unwrap()
    .board
    .token
}

/// Four users who all added `[START, START + 1h)`, a second apart, so a game formed with them in
/// that order.
async fn formed() -> (Store, SqlitePool, String, [String; 4]) {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let c = user(&s, "Marta").await;
    let d = user(&s, "Joana").await;
    let token = board(&s, &a).await;
    for (i, who) in [&a, &b, &c, &d].into_iter().enumerate() {
        let now = T0 + i64::try_from(i).unwrap();
        s.add_free(&token, who, START, START + HOUR, now)
            .await
            .unwrap();
    }
    (s, pool, token, [a, b, c, d])
}

/// Two overlapping full games, `g1` at `START` and `g2` half an hour later, with their players.
/// The first holds `a1..a4`, the second `b1..b4`.
async fn two_games() -> (Store, SqlitePool, String, [String; 4], [String; 4]) {
    let (s, pool) = setup().await;
    let mut first = Vec::new();
    let mut second = Vec::new();
    for i in 0..4 {
        first.push(user(&s, &format!("Aa{i}")).await);
    }
    for i in 0..4 {
        second.push(user(&s, &format!("Bb{i}")).await);
    }
    let token = board(&s, &first[0]).await;
    for (i, who) in first.iter().enumerate() {
        let now = T0 + i64::try_from(i).unwrap();
        s.add_free(&token, who, START, START + HOUR, now)
            .await
            .unwrap();
    }
    for (i, who) in second.iter().enumerate() {
        let now = T0 + 10 + i64::try_from(i).unwrap();
        s.add_free(&token, who, START + HALF, START + HALF + HOUR, now)
            .await
            .unwrap();
    }
    let first: [String; 4] = first.try_into().unwrap();
    let second: [String; 4] = second.try_into().unwrap();
    (s, pool, token, first, second)
}

async fn view(s: &Store, token: &str, who: &str, now: i64) -> BoardView {
    s.board_view(token, Some(who), now).await.unwrap().unwrap()
}

fn players(view: &BoardView, game: usize) -> Vec<String> {
    view.games[game]
        .players
        .iter()
        .map(|p| p.user_id.clone())
        .collect()
}

async fn outbox(pool: &SqlitePool) -> Vec<(String, Value)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, payload FROM push_outbox ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    rows.into_iter()
        .map(|(u, p)| (u, serde_json::from_str(&p).unwrap()))
        .collect()
}

fn recipients(rows: &[(String, Value)], kind: &str) -> Vec<String> {
    let mut users: Vec<String> = rows
        .iter()
        .filter(|(_, p)| p["kind"] == kind)
        .map(|(u, _)| u.clone())
        .collect();
    users.sort();
    users
}

fn sorted(users: &[&String]) -> Vec<String> {
    let mut out: Vec<String> = users.iter().map(|u| (*u).clone()).collect();
    out.sort();
    out
}

async fn clear_outbox(pool: &SqlitePool) {
    sqlx::query("DELETE FROM push_outbox")
        .execute(pool)
        .await
        .unwrap();
}

fn code(err: StoreError) -> &'static str {
    match err {
        StoreError::Conflict(code) => code,
        StoreError::NotFound => "not_found",
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn leave_fills_from_the_line_and_tells_everyone() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Eva").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 10)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    let v = view(&s, &token, &b, T0 + 20).await;
    assert_eq!(v.games[0].line, vec![e.clone()]);
    let version = v.games[0].version;
    let after = s
        .leave_game(&token, &v.games[0].id, &b, version, T0 + 20)
        .await
        .unwrap();
    // Eva takes Rui's seat.
    assert_eq!(
        players(&after, 0),
        vec![a.clone(), e.clone(), c.clone(), d.clone()]
    );
    assert!(after.games[0].version > version);
    assert_eq!(after.games[0].state, GameState::Full);
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "player_left"), sorted(&[&a, &c, &d]));
    assert_eq!(recipients(&rows, "moved_in"), vec![e]);
    let left = rows
        .iter()
        .find(|(_, p)| p["kind"] == "player_left")
        .unwrap();
    assert_eq!(left.1["name"], "Rui");
}

#[tokio::test]
async fn leave_inside_one_hour_leaves_the_game_short() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Eva").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 10)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    let now = START - HALF;
    let v = view(&s, &token, &b, now).await;
    let after = s
        .leave_game(&token, &v.games[0].id, &b, v.games[0].version, now)
        .await
        .unwrap();
    assert_eq!(players(&after, 0), vec![a.clone(), c.clone(), d.clone()]);
    assert_eq!(after.games[0].state, GameState::Short);
    // Eva is first in line but is not moved in by herself.
    assert_eq!(after.games[0].line, vec![e]);
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "player_left"), sorted(&[&a, &c, &d]));
    assert!(recipients(&rows, "moved_in").is_empty());
}

#[tokio::test]
async fn last_player_leaving_deletes_the_game() {
    let (s, _, token, who) = formed().await;
    // Inside the hour nobody fills in, so the game just empties.
    let now = START - HALF;
    let mut last = None;
    for user in &who {
        let v = view(&s, &token, user, now).await;
        last = Some(
            s.leave_game(&token, &v.games[0].id, user, v.games[0].version, now)
                .await
                .unwrap_or_else(|e| panic!("{user}: {e:?}")),
        );
        if user != &who[3] {
            assert!(!last.as_ref().unwrap().games.is_empty());
        }
    }
    assert!(last.unwrap().games.is_empty());
}

#[tokio::test]
async fn leaver_is_not_put_back() {
    let (s, _, token, [a, b, c, d]) = formed().await;
    let v = view(&s, &token, &b, T0 + 20).await;
    let after = s
        .leave_game(&token, &v.games[0].id, &b, v.games[0].version, T0 + 20)
        .await
        .unwrap();
    // Nobody else is free, so the game is short, and Rui's free time over it is gone.
    assert_eq!(players(&after, 0), vec![a, c, d]);
    assert!(after.pieces.iter().all(|p| p.user_id != b));
    assert_eq!(after.games[0].state, GameState::Short);
}

#[tokio::test]
async fn leave_refusals() {
    let (s, _, token, [a, b, ..]) = formed().await;
    let outsider = user(&s, "Zed").await;
    let v = view(&s, &token, &a, T0 + 20).await;
    let (g, ver) = (v.games[0].id.clone(), v.games[0].version);
    let err = s.leave_game(&token, &g, &outsider, ver, T0 + 20).await;
    assert_eq!(code(err.unwrap_err()), "not_in_game");
    let err = s.leave_game(&token, "nope", &a, ver, T0 + 20).await;
    assert_eq!(code(err.unwrap_err()), "not_found");
    let err = s.leave_game("nope", &g, &a, ver, T0 + 20).await;
    assert_eq!(code(err.unwrap_err()), "not_found");
    let err = s.leave_game(&token, &g, &b, ver, START).await;
    assert_eq!(code(err.unwrap_err()), "game_started");
    let err = s.leave_game(&token, &g, &b, ver, START + WEEK_SECS).await;
    assert_eq!(code(err.unwrap_err()), "board_over");
    let err = s.leave_game(&token, &g, &b, ver + 1, T0 + 20).await;
    assert_eq!(code(err.unwrap_err()), "game_changed");
}

#[tokio::test]
async fn stale_game_version_is_refused() {
    let (s, pool, token, [a, ..]) = formed().await;
    let e = user(&s, "Eva").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 10)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    let v = view(&s, &token, &a, T0 + 20).await;
    let err = s
        .give_place(&token, &v.games[0].id, &a, v.games[0].version - 1, T0 + 20)
        .await
        .unwrap_err();
    assert_eq!(code(err), "game_changed");
    let after = view(&s, &token, &a, T0 + 20).await;
    assert_eq!(players(&after, 0), players(&v, 0));
    assert_eq!(after.games[0].version, v.games[0].version);
    assert!(outbox(&pool).await.is_empty());
    for action in ["leave", "take"] {
        let r = match action {
            "leave" => s.leave_game(&token, &v.games[0].id, &a, 99, T0 + 20).await,
            _ => s.take_place(&token, &v.games[0].id, &e, 99, T0 + 20).await,
        };
        assert_eq!(code(r.unwrap_err()), "game_changed", "{action}");
    }
}

#[tokio::test]
async fn give_trade_moves_both_and_queues_place_given() {
    let (s, pool, token, first, second) = two_games().await;
    let [a0, a1, a2, a3] = &first;
    let [b0, b1, ..] = &second;
    // a0 also covers the second game; b0 also covers the first. b0 is the oldest in game two.
    s.add_free(&token, a0, START + HOUR, START + HOUR + HALF, T0 + 20)
        .await
        .unwrap();
    s.add_free(&token, b0, START, START + HALF, T0 + 21)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    let v = view(&s, &token, a0, T0 + 30).await;
    assert_eq!(v.games.len(), 2);
    let (g1, g2) = (&v.games[0], &v.games[1]);
    assert_eq!(players(&v, 0), first.to_vec());
    assert_eq!(players(&v, 1), second.to_vec());
    let after = s
        .give_place(&token, &g1.id, a0, g1.version, T0 + 30)
        .await
        .unwrap();
    // a0 and b0 swap seats, each keeping the seat they take over.
    assert_eq!(
        players(&after, 0),
        vec![b0.clone(), a1.clone(), a2.clone(), a3.clone()]
    );
    assert_eq!(
        players(&after, 1),
        vec![a0.clone(), b1.clone(), second[2].clone(), second[3].clone()]
    );
    assert_eq!(after.games[0].version, g1.version + 1);
    assert_eq!(after.games[1].version, g2.version + 1);
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "place_given"), vec![b0.clone()]);
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "place_given")
        .unwrap();
    assert_eq!(p["name"], "Aa0");
    assert_eq!(p["start"], g2.start);
    assert_eq!(p["end"], g2.end);
    assert_eq!(p["new_start"], g1.start);
    assert_eq!(p["new_end"], g1.end);
    assert_eq!(p["url"], format!("/b/{token}"));
    assert!(recipients(&rows, "moved_in").is_empty());
}

#[tokio::test]
async fn give_to_the_line_heads_it_and_refusals() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    // Nobody to take the place yet.
    let v = view(&s, &token, &a, T0 + 20).await;
    let err = s
        .give_place(&token, &v.games[0].id, &a, v.games[0].version, T0 + 20)
        .await
        .unwrap_err();
    assert_eq!(code(err), "nobody_can_take");
    // Inside the hour it is too late.
    let e = user(&s, "Eva").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 30)
        .await
        .unwrap();
    let late = START - HALF;
    let v = view(&s, &token, &a, late).await;
    let err = s
        .give_place(&token, &v.games[0].id, &a, v.games[0].version, late)
        .await
        .unwrap_err();
    assert_eq!(code(err), "too_late");
    // An outsider is not in the game.
    let err = s
        .give_place(&token, &v.games[0].id, &e, v.games[0].version, T0 + 40)
        .await
        .unwrap_err();
    assert_eq!(code(err), "not_in_game");
    clear_outbox(&pool).await;
    let v = view(&s, &token, &a, T0 + 40).await;
    let after = s
        .give_place(&token, &v.games[0].id, &a, v.games[0].version, T0 + 40)
        .await
        .unwrap();
    assert_eq!(
        players(&after, 0),
        vec![e.clone(), b.clone(), c.clone(), d.clone()]
    );
    // Ana heads the line now, and her free time is untouched.
    assert_eq!(after.games[0].line, vec![a.clone()]);
    assert!(after.pieces.iter().any(|p| p.user_id == a));
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "moved_in"), vec![e]);
}

#[tokio::test]
async fn line_head_comes_first_when_a_place_opens() {
    let (s, _, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Eva").await;
    let f = user(&s, "Filipe").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 10)
        .await
        .unwrap();
    s.add_free(&token, &f, START, START + HOUR, T0 + 11)
        .await
        .unwrap();
    let v = view(&s, &token, &a, T0 + 20).await;
    s.give_place(&token, &v.games[0].id, &a, v.games[0].version, T0 + 20)
        .await
        .unwrap();
    // Eva took Ana's place. Now Rui leaves: Ana heads the line, ahead of Filipe.
    let v = view(&s, &token, &b, T0 + 30).await;
    assert_eq!(v.games[0].line, vec![a.clone(), f.clone()]);
    let after = s
        .leave_game(&token, &v.games[0].id, &b, v.games[0].version, T0 + 30)
        .await
        .unwrap();
    assert_eq!(players(&after, 0), vec![e, a, c, d]);
}

#[tokio::test]
async fn take_open_place_inside_the_hour() {
    let (s, _, token, [a, b, c, d]) = formed().await;
    let now = START - HALF;
    let v = view(&s, &token, &b, now).await;
    s.leave_game(&token, &v.games[0].id, &b, v.games[0].version, now)
        .await
        .unwrap();
    let z = user(&s, "Zed").await;
    let v = view(&s, &token, &z, now).await;
    assert_eq!(v.games[0].state, GameState::Short);
    let after = s
        .take_place(&token, &v.games[0].id, &z, v.games[0].version, now)
        .await
        .unwrap();
    assert_eq!(players(&after, 0), vec![a, z.clone(), c, d]);
    assert_eq!(after.games.len(), 1);
    assert_eq!(after.games[0].state, GameState::Full);
    assert!(after.me.unwrap().member);
    // Zed's free time over the game is added, created now.
    let mine: Vec<_> = after.pieces.iter().filter(|p| p.user_id == z).collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(
        (mine[0].start, mine[0].end, mine[0].created_at),
        (START, START + HOUR, now)
    );
    // Taking again, or taking a full game, is refused.
    let v = view(&s, &token, &z, now).await;
    let err = s
        .take_place(&token, &v.games[0].id, &z, v.games[0].version, now)
        .await
        .unwrap_err();
    assert_eq!(code(err), "already_in_game");
    let other = user(&s, "Yan").await;
    let err = s
        .take_place(&token, &v.games[0].id, &other, v.games[0].version, now)
        .await
        .unwrap_err();
    assert_eq!(code(err), "game_full");
    let err = s
        .take_place(&token, &v.games[0].id, &other, v.games[0].version, START)
        .await
        .unwrap_err();
    assert_eq!(code(err), "game_started");
}

#[tokio::test]
async fn take_refused_when_busy_in_another_game() {
    let (s, _, token, first, second) = two_games().await;
    let now = START - HALF;
    // Open a place in the first game, then a player of the second tries to take it.
    let v = view(&s, &token, &first[0], now).await;
    s.leave_game(&token, &v.games[0].id, &first[0], v.games[0].version, now)
        .await
        .unwrap();
    let v = view(&s, &token, &second[0], now).await;
    assert_eq!(v.games[0].state, GameState::Short);
    let err = s
        .take_place(&token, &v.games[0].id, &second[0], v.games[0].version, now)
        .await
        .unwrap_err();
    assert_eq!(code(err), "busy");
    let after = view(&s, &token, &second[0], now).await;
    assert_eq!(players(&after, 0), players(&v, 0));
}
