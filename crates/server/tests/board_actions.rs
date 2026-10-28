#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Game actions on a board: Leave, Give my place and Take the open place, through the store.

mod support;

use axum::http::{Method, StatusCode};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{BoardView, GameState, NewBoard, RequestKind, WEEK_SECS};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
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

/// `two_games` with `a0` also free for the second game and `b0` for the first, so they can swap.
async fn swappable() -> (Store, SqlitePool, String, [String; 4], [String; 4]) {
    let (s, pool, token, first, second) = two_games().await;
    s.add_free(
        &token,
        &first[0],
        START + HOUR,
        START + HOUR + HALF,
        T0 + 20,
    )
    .await
    .unwrap();
    s.add_free(&token, &second[0], START, START + HALF, T0 + 21)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    (s, pool, token, first, second)
}

async fn request_status(pool: &SqlitePool, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM board_requests WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Ana asks Rui... the first request in `who`'s view.
fn first_request(v: &BoardView) -> &padel_score_server::boards::RequestOut {
    v.requests.first().expect("a pending request")
}

#[tokio::test]
async fn swap_accept_trades_places() {
    let (s, pool, token, first, second) = swappable().await;
    let [a0, a1, a2, a3] = &first;
    let [b0, b1, b2, b3] = &second;
    let v = view(&s, &token, a0, T0 + 30).await;
    let (g1, g2) = (&v.games[0], &v.games[1]);
    let asked = s
        .ask_swap(&token, &g1.id, a0, g1.version, &g2.id, b0, T0 + 30)
        .await
        .unwrap();
    let mine = first_request(&asked);
    assert_eq!(mine.kind, RequestKind::Swap);
    assert!(!mine.needs_my_answer);
    assert_eq!(mine.from.user_id, *a0);
    assert_eq!(mine.game_id, g1.id);
    assert_eq!(mine.other_game_id.as_deref(), Some(g2.id.as_str()));
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "swap_asked"), vec![b0.clone()]);
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "swap_asked")
        .unwrap();
    assert_eq!(p["name"], "Aa0");
    assert_eq!(
        (p["start"].as_i64(), p["end"].as_i64()),
        (Some(g2.start), Some(g2.end))
    );
    assert_eq!(
        (p["new_start"].as_i64(), p["new_end"].as_i64()),
        (Some(g1.start), Some(g1.end))
    );
    // A second ask from the same user on the same game is refused.
    let err = s
        .ask_swap(&token, &g1.id, a0, g1.version, &g2.id, b0, T0 + 31)
        .await
        .unwrap_err();
    assert_eq!(code(err), "request_pending");
    // The addressee sees it and must answer; bystanders see nothing.
    let theirs = view(&s, &token, b0, T0 + 32).await;
    assert!(first_request(&theirs).needs_my_answer);
    assert!(view(&s, &token, a1, T0 + 32).await.requests.is_empty());
    clear_outbox(&pool).await;
    let after = s.answer_request(&mine.id, b0, true, T0 + 40).await.unwrap();
    assert_eq!(
        players(&after, 0),
        vec![b0.clone(), a1.clone(), a2.clone(), a3.clone()]
    );
    assert_eq!(
        players(&after, 1),
        vec![a0.clone(), b1.clone(), b2.clone(), b3.clone()]
    );
    assert_eq!(after.games[0].version, g1.version + 1);
    assert_eq!(after.games[1].version, g2.version + 1);
    assert!(after.requests.is_empty());
    assert_eq!(request_status(&pool, &mine.id).await, "accepted");
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "swap_answered"), vec![a0.clone()]);
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "swap_answered")
        .unwrap();
    assert_eq!(p["accepted"], true);
    assert_eq!(p["name"], "Bb0");
    assert_eq!(
        (p["start"].as_i64(), p["end"].as_i64()),
        (Some(g1.start), Some(g1.end))
    );
    assert_eq!(
        (p["new_start"].as_i64(), p["new_end"].as_i64()),
        (Some(g2.start), Some(g2.end))
    );
    // Answered once: a second answer finds nothing pending.
    let err = s
        .answer_request(&mine.id, b0, true, T0 + 41)
        .await
        .unwrap_err();
    assert_eq!(code(err), "not_found");
}

#[tokio::test]
async fn swap_asks_are_refused_when_they_cannot_work() {
    let (s, _, token, first, second) = swappable().await;
    let v = view(&s, &token, &first[0], T0 + 30).await;
    let (g1, g2) = (&v.games[0], &v.games[1]);
    // Rui (a1) is not free for the second game.
    let err = s
        .ask_swap(
            &token,
            &g1.id,
            &first[1],
            g1.version,
            &g2.id,
            &second[0],
            T0 + 30,
        )
        .await
        .unwrap_err();
    assert_eq!(code(err), "cannot_swap");
    // A stale version is refused first.
    let err = s
        .ask_swap(
            &token,
            &g1.id,
            &first[0],
            g1.version + 1,
            &g2.id,
            &second[0],
            T0 + 30,
        )
        .await
        .unwrap_err();
    assert_eq!(code(err), "game_changed");
    let err = s
        .ask_swap(
            &token,
            &g1.id,
            &first[0],
            g1.version,
            "nope",
            &second[0],
            T0 + 30,
        )
        .await
        .unwrap_err();
    assert_eq!(code(err), "not_found");
}

#[tokio::test]
async fn swap_decline_changes_nothing() {
    let (s, pool, token, first, second) = swappable().await;
    let b0 = &second[0];
    let v = view(&s, &token, &first[0], T0 + 30).await;
    let (g1, g2) = (&v.games[0], &v.games[1]);
    let asked = s
        .ask_swap(&token, &g1.id, &first[0], g1.version, &g2.id, b0, T0 + 30)
        .await
        .unwrap();
    clear_outbox(&pool).await;
    let id = first_request(&asked).id.clone();
    let after = s.answer_request(&id, b0, false, T0 + 40).await.unwrap();
    assert_eq!(players(&after, 0), first.to_vec());
    assert_eq!(players(&after, 1), second.to_vec());
    assert_eq!(after.games[0].version, g1.version);
    assert_eq!(after.games[1].version, g2.version);
    assert!(after.requests.is_empty());
    assert_eq!(request_status(&pool, &id).await, "declined");
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "swap_answered"), vec![first[0].clone()]);
    assert_eq!(rows[0].1["accepted"], false);
}

#[tokio::test]
async fn swap_request_expires_when_a_game_changes() {
    let (s, pool, token, first, second) = swappable().await;
    let b0 = &second[0];
    let v = view(&s, &token, &first[0], T0 + 30).await;
    let (g1, g2) = (&v.games[0], &v.games[1]);
    let asked = s
        .ask_swap(&token, &g1.id, &first[0], g1.version, &g2.id, b0, T0 + 30)
        .await
        .unwrap();
    let id = first_request(&asked).id.clone();
    // Someone leaves the other game.
    s.leave_game(&token, &g2.id, &second[1], g2.version, T0 + 31)
        .await
        .unwrap();
    // The stale request is hidden on read.
    assert!(view(&s, &token, b0, T0 + 32).await.requests.is_empty());
    assert!(
        view(&s, &token, &first[0], T0 + 32)
            .await
            .requests
            .is_empty()
    );
    let err = s.answer_request(&id, b0, true, T0 + 33).await.unwrap_err();
    assert_eq!(code(err), "request_expired");
    assert_eq!(request_status(&pool, &id).await, "expired");
    let after = view(&s, &token, b0, T0 + 34).await;
    assert_eq!(players(&after, 0), first.to_vec());
}

#[tokio::test]
async fn swap_that_no_longer_works_expires_with_its_code() {
    let (s, pool, token, first, second) = swappable().await;
    let (a0, b0) = (&first[0], &second[0]);
    let v = view(&s, &token, a0, T0 + 30).await;
    let (g1, g2) = (&v.games[0], &v.games[1]);
    let asked = s
        .ask_swap(&token, &g1.id, a0, g1.version, &g2.id, b0, T0 + 30)
        .await
        .unwrap();
    let id = first_request(&asked).id.clone();
    // Ana drops her free time over the second game; no game version changes.
    s.remove_free(&token, a0, START + HOUR, START + HOUR + HALF, T0 + 31)
        .await
        .unwrap();
    let err = s.answer_request(&id, b0, true, T0 + 32).await.unwrap_err();
    assert_eq!(code(err), "cannot_swap");
    assert_eq!(request_status(&pool, &id).await, "expired");
    assert_eq!(
        players(&view(&s, &token, b0, T0 + 33).await, 0),
        first.to_vec()
    );
}

/// Ask for START + 2h .. + 3h from `who[0]` in a formed game.
async fn ask_later(s: &Store, token: &str, who: &[String; 4], agreed: bool, now: i64) -> BoardView {
    let v = view(s, token, &who[0], now).await;
    s.ask_time(
        token,
        &v.games[0].id,
        &who[0],
        v.games[0].version,
        START + 2 * HOUR,
        START + 3 * HOUR,
        agreed,
        now,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn time_change_applies_when_all_accept_and_marks_moved() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let before = view(&s, &token, &a, T0 + 20).await.games[0].clone();
    clear_outbox(&pool).await;
    let asked = ask_later(
        &s,
        &token,
        &[a.clone(), b.clone(), c.clone(), d.clone()],
        false,
        T0 + 20,
    )
    .await;
    let req = first_request(&asked).clone();
    assert_eq!(req.kind, RequestKind::Time);
    assert_eq!(req.new_start, Some(START + 2 * HOUR));
    assert_eq!(req.new_end, Some(START + 3 * HOUR));
    assert!(!req.needs_my_answer);
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "time_asked"), sorted(&[&b, &c, &d]));
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "time_asked")
        .unwrap();
    assert_eq!(p["name"], "Ana");
    assert_eq!(p["start"], before.start);
    assert_eq!(p["new_start"], START + 2 * HOUR);
    // Two accept: still waiting. Accepting twice is refused.
    let mid = s.answer_request(&req.id, &b, true, T0 + 30).await.unwrap();
    assert_eq!(mid.games[0].start, before.start);
    assert!(!mid.games[0].moved);
    assert!(!first_request(&mid).needs_my_answer);
    let err = s
        .answer_request(&req.id, &b, true, T0 + 31)
        .await
        .unwrap_err();
    assert_eq!(code(err), "already_answered");
    s.answer_request(&req.id, &c, true, T0 + 32).await.unwrap();
    clear_outbox(&pool).await;
    // The last one makes it happen.
    let after = s.answer_request(&req.id, &d, true, T0 + 33).await.unwrap();
    let g = &after.games[0];
    assert_eq!((g.start, g.end), (START + 2 * HOUR, START + 3 * HOUR));
    assert!(g.moved);
    assert!(g.version > before.version);
    assert_eq!(
        players(&after, 0),
        vec![a.clone(), b.clone(), c.clone(), d.clone()]
    );
    assert!(after.requests.is_empty());
    assert_eq!(request_status(&pool, &req.id).await, "accepted");
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "time_answered"), vec![a.clone()]);
    assert_eq!(recipients(&rows, "time_changed"), sorted(&[&b, &c]));
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "time_answered")
        .unwrap();
    assert_eq!(p["accepted"], true);
    let (_, p) = rows
        .iter()
        .find(|(_, p)| p["kind"] == "time_changed")
        .unwrap();
    assert_eq!(p["name"], "Ana");
    assert_eq!(
        (p["start"].as_i64(), p["new_start"].as_i64()),
        (Some(before.start), Some(START + 2 * HOUR))
    );
}

#[tokio::test]
async fn one_decline_drops_the_time_change() {
    let (s, pool, token, who) = formed().await;
    let [a, b, c, _] = &who;
    let before = view(&s, &token, a, T0 + 20).await.games[0].clone();
    let asked = ask_later(&s, &token, &who, false, T0 + 20).await;
    let id = first_request(&asked).id.clone();
    clear_outbox(&pool).await;
    let after = s.answer_request(&id, b, false, T0 + 30).await.unwrap();
    assert_eq!(after.games[0].start, before.start);
    assert!(!after.games[0].moved);
    assert_eq!(after.games[0].version, before.version);
    assert!(after.requests.is_empty());
    assert_eq!(request_status(&pool, &id).await, "declined");
    let rows = outbox(&pool).await;
    assert_eq!(recipients(&rows, "time_answered"), vec![a.clone()]);
    assert_eq!(rows[0].1["accepted"], false);
    assert_eq!(rows[0].1["name"], "Rui");
    // The request is closed for everyone else.
    let err = s.answer_request(&id, c, true, T0 + 31).await.unwrap_err();
    assert_eq!(code(err), "not_found");
}

#[tokio::test]
async fn agreed_in_the_chat_applies_at_once_and_only_tells() {
    let (s, pool, token, who) = formed().await;
    let [a, b, c, d] = &who;
    let before = view(&s, &token, a, T0 + 20).await.games[0].clone();
    clear_outbox(&pool).await;
    let after = ask_later(&s, &token, &who, true, T0 + 20).await;
    let g = &after.games[0];
    assert_eq!((g.start, g.end), (START + 2 * HOUR, START + 3 * HOUR));
    assert!(g.moved);
    assert!(g.version > before.version);
    assert!(after.requests.is_empty());
    let rows = outbox(&pool).await;
    assert!(recipients(&rows, "time_asked").is_empty());
    assert_eq!(recipients(&rows, "time_changed"), sorted(&[b, c, d]));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_lone_player_changes_the_time_at_once() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    // Everyone else leaves; the game is short inside the hour so nobody fills in.
    let now = START - 2 * HOUR;
    for who in [&b, &c, &d] {
        let v = view(&s, &token, who, now).await;
        s.leave_game(&token, &v.games[0].id, who, v.games[0].version, now)
            .await
            .unwrap();
    }
    clear_outbox(&pool).await;
    let v = view(&s, &token, &a, now).await;
    assert_eq!(v.games[0].players.len(), 1);
    let after = s
        .ask_time(
            &token,
            &v.games[0].id,
            &a,
            v.games[0].version,
            START + 2 * HOUR,
            START + 3 * HOUR,
            false,
            now,
        )
        .await
        .unwrap();
    assert!(after.games[0].moved);
    assert_eq!(after.games[0].start, START + 2 * HOUR);
    assert!(after.requests.is_empty());
    assert!(outbox(&pool).await.is_empty());
}

#[tokio::test]
async fn moved_game_keeps_players_without_ranges_at_the_new_time() {
    let (s, _, token, who) = formed().await;
    let after = ask_later(&s, &token, &who, true, T0 + 20).await;
    // Nobody has free time over the new time, yet a later settle keeps them in.
    assert!(after.pieces.is_empty());
    let z = user(&s, "Zed").await;
    let later = s
        .add_free(&token, &z, START + 5 * HOUR, START + 6 * HOUR, T0 + 30)
        .await
        .unwrap();
    assert_eq!(later.games.len(), 1);
    assert_eq!(players(&later, 0), who.to_vec());
    assert!(later.games[0].moved);
}

#[tokio::test]
async fn moved_game_does_not_reform_at_the_old_time() {
    let (s, _, token, who) = formed().await;
    let after = ask_later(&s, &token, &who, true, T0 + 20).await;
    // The four are not free at the old time any more, so no second game forms there.
    assert_eq!(after.games.len(), 1);
    assert_eq!(after.games[0].start, START + 2 * HOUR);
    assert!(
        after
            .pieces
            .iter()
            .all(|p| p.end <= START || p.start >= START + HOUR)
    );
    // An overlapping move keeps the shared part of their free time.
    let (s, _, token, who) = formed().await;
    let v = view(&s, &token, &who[0], T0 + 20).await;
    let after = s
        .ask_time(
            &token,
            &v.games[0].id,
            &who[0],
            v.games[0].version,
            START + HALF,
            START + HALF + HOUR,
            true,
            T0 + 20,
        )
        .await
        .unwrap();
    assert_eq!(after.games.len(), 1);
    assert_eq!(after.pieces.len(), 4);
    assert!(
        after
            .pieces
            .iter()
            .all(|p| (p.start, p.end) == (START + HALF, START + HOUR))
    );
}

#[tokio::test]
async fn time_change_refusals() {
    let (s, _, token, who) = formed().await;
    let v = view(&s, &token, &who[0], T0 + 20).await;
    let (g, ver) = (v.games[0].id.clone(), v.games[0].version);
    let ask = |start: i64, end: i64, now: i64, user: &String| {
        let (s, token, g) = (s.clone(), token.clone(), g.clone());
        let user = user.clone();
        async move {
            s.ask_time(&token, &g, &user, ver, start, end, false, now)
                .await
        }
    };
    let err = ask(START + 2 * HOUR, START + 2 * HOUR + HALF, T0 + 20, &who[0]).await;
    assert!(matches!(err, Err(StoreError::InvalidField("range"))));
    let err = ask(START, START + HOUR + HALF, START - HALF, &who[0]).await;
    assert_eq!(code(err.unwrap_err()), "too_soon");
    let outsider = user(&s, "Zed").await;
    let err = ask(START + 2 * HOUR, START + 3 * HOUR, T0 + 20, &outsider).await;
    assert_eq!(code(err.unwrap_err()), "not_in_game");
    // A pending request blocks a second one from the same player.
    ask(START + 2 * HOUR, START + 3 * HOUR, T0 + 20, &who[0])
        .await
        .unwrap();
    let err = ask(START + 3 * HOUR, START + 4 * HOUR, T0 + 21, &who[0]).await;
    assert_eq!(code(err.unwrap_err()), "request_pending");
}

#[tokio::test]
async fn a_time_change_to_the_games_own_time_is_refused() {
    let (s, pool, token, who) = formed().await;
    let v = view(&s, &token, &who[0], T0 + 20).await;
    let (g, ver) = (v.games[0].id.clone(), v.games[0].version);
    clear_outbox(&pool).await;
    for agreed in [false, true] {
        let err = s
            .ask_time(
                &token,
                &g,
                &who[0],
                ver,
                START,
                START + HOUR,
                agreed,
                T0 + 20,
            )
            .await;
        assert!(matches!(err, Err(StoreError::InvalidField("range"))));
    }
    // Nothing moved, nobody was told, and the game is as it was.
    let after = view(&s, &token, &who[0], T0 + 21).await;
    assert_eq!(after.games[0].version, ver);
    assert!(!after.games[0].moved);
    assert!(outbox(&pool).await.is_empty());
}

#[tokio::test]
async fn answering_after_the_game_started_is_request_expired_and_nobody_moves() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let asked = ask_later(
        &s,
        &token,
        &[a.clone(), b.clone(), c.clone(), d.clone()],
        false,
        T0 + 20,
    )
    .await;
    let id = first_request(&asked).id.clone();
    let err = s
        .answer_request(&id, &b, true, START + 1)
        .await
        .unwrap_err();
    assert_eq!(code(err), "request_expired");
    assert_eq!(request_status(&pool, &id).await, "expired");
    let g = &view(&s, &token, &a, START + 2).await.games[0];
    assert!(!g.moved);
    assert_eq!((g.start, g.end), (START, START + HOUR));
}

#[tokio::test]
async fn a_time_request_whose_slot_filled_since_is_players_busy_and_expires() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let asked = ask_later(
        &s,
        &token,
        &[a.clone(), b.clone(), c.clone(), d.clone()],
        false,
        T0 + 20,
    )
    .await;
    let id = first_request(&asked).id.clone();
    // Rui and three newcomers are free then, so a game forms at the proposed time with Rui in it.
    for (i, who) in [b.clone()].into_iter().enumerate() {
        s.add_free(
            &token,
            &who,
            START + 2 * HOUR,
            START + 3 * HOUR,
            T0 + 21 + i as i64,
        )
        .await
        .unwrap();
    }
    for (i, name) in ["Zed", "Eva", "Tom"].into_iter().enumerate() {
        let u = user(&s, name).await;
        s.add_free(
            &token,
            &u,
            START + 2 * HOUR,
            START + 3 * HOUR,
            T0 + 30 + i as i64,
        )
        .await
        .unwrap();
    }
    assert_eq!(view(&s, &token, &a, T0 + 40).await.games.len(), 2);
    s.answer_request(&id, &c, true, T0 + 41).await.unwrap();
    s.answer_request(&id, &d, true, T0 + 42).await.unwrap();
    let err = s.answer_request(&id, &b, true, T0 + 43).await.unwrap_err();
    assert_eq!(code(err), "players_busy");
    assert_eq!(request_status(&pool, &id).await, "expired");
    let after = view(&s, &token, &a, T0 + 44).await;
    let g = after
        .games
        .iter()
        .find(|g| g.start == START)
        .expect("the game stays");
    assert!(!g.moved);
}

#[tokio::test]
async fn a_time_request_staled_by_a_leave_is_request_expired() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let asked = ask_later(
        &s,
        &token,
        &[a.clone(), b.clone(), c.clone(), d.clone()],
        false,
        T0 + 20,
    )
    .await;
    let id = first_request(&asked).id.clone();
    let g = view(&s, &token, &c, T0 + 21).await.games[0].clone();
    s.leave_game(&token, &g.id, &c, g.version, T0 + 22)
        .await
        .unwrap();
    let err = s.answer_request(&id, &b, true, T0 + 23).await.unwrap_err();
    assert_eq!(code(err), "request_expired");
    assert_eq!(request_status(&pool, &id).await, "expired");
}

#[tokio::test]
async fn only_addressees_can_answer() {
    let (s, _, token, who) = formed().await;
    let asked = ask_later(&s, &token, &who, false, T0 + 20).await;
    let id = first_request(&asked).id.clone();
    // The asker, a stranger and an unknown request all get 404.
    let stranger = user(&s, "Zed").await;
    for user in [&who[0], &stranger] {
        let err = s
            .answer_request(&id, user, true, T0 + 30)
            .await
            .unwrap_err();
        assert_eq!(code(err), "not_found");
    }
    let err = s
        .answer_request("nope", &who[1], true, T0 + 30)
        .await
        .unwrap_err();
    assert_eq!(code(err), "not_found");
    // A swap's addressee is only the asked player, not the rest of their game.
    let (s, _, token, first, second) = swappable().await;
    let v = view(&s, &token, &first[0], T0 + 30).await;
    let asked = s
        .ask_swap(
            &token,
            &v.games[0].id,
            &first[0],
            v.games[0].version,
            &v.games[1].id,
            &second[0],
            T0 + 30,
        )
        .await
        .unwrap();
    let id = first_request(&asked).id.clone();
    for user in [&second[1], &first[1], &first[0]] {
        let err = s
            .answer_request(&id, user, true, T0 + 31)
            .await
            .unwrap_err();
        assert_eq!(code(err), "not_found");
    }
    assert!(
        view(&s, &token, &second[1], T0 + 31)
            .await
            .requests
            .is_empty()
    );
}

#[tokio::test]
async fn leave_is_never_refused_for_the_piece_cap() {
    let (s, pool) = setup().await;
    let ids = [
        user(&s, "Ana").await,
        user(&s, "Rui").await,
        user(&s, "Marta").await,
        user(&s, "Joana").await,
    ];
    let token = board(&s, &ids[0]).await;
    let (from, to) = (START + 3 * HOUR, START + 4 * HOUR);
    for (i, who) in [&ids[0], &ids[2], &ids[3]].into_iter().enumerate() {
        s.add_free(&token, who, from, to, T0 + i64::try_from(i).unwrap())
            .await
            .unwrap();
    }
    // Rui's one piece spans the whole game and more.
    s.add_free(&token, &ids[1], START + 2 * HOUR, START + 5 * HOUR, T0 + 3)
        .await
        .unwrap();
    let board_id: String = sqlx::query_scalar("SELECT id FROM boards WHERE token = ?")
        .bind(&token)
        .fetch_one(&pool)
        .await
        .unwrap();
    // Fill him up to the cap with half-hour pieces further on.
    for i in 0..199_i64 {
        sqlx::query(
            "INSERT INTO board_ranges (id, board_id, user_id, starts_at, ends_at, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("fill-{i}"))
        .bind(&board_id)
        .bind(&ids[1])
        .bind(START + 6 * HOUR + i * HALF)
        .bind(START + 6 * HOUR + (i + 1) * HALF)
        .bind(T0 + 4)
        .execute(&pool)
        .await
        .unwrap();
    }
    let v = view(&s, &token, &ids[1], T0 + 20).await;
    assert_eq!(v.games.len(), 1);
    assert_eq!(players(&v, 0).len(), 4);
    // Cutting the game out of the long piece makes two pieces: over the cap, but Leave goes through.
    let after = s
        .leave_game(&token, &v.games[0].id, &ids[1], v.games[0].version, T0 + 20)
        .await
        .unwrap();
    assert_eq!(after.games[0].players.len(), 3);
    assert!(after.games[0].players.iter().all(|p| p.user_id != ids[1]));
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_ranges WHERE user_id = ?")
        .bind(&ids[1])
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 201);
}

// Start match from a game.

/// The first game's id on the board.
async fn first_game(s: &Store, token: &str, who: &str, now: i64) -> String {
    view(s, token, who, now).await.games[0].id.clone()
}

#[tokio::test]
async fn start_match_window_opens_thirty_minutes_before() {
    let (s, _pool, token, [a, ..]) = formed().await;
    let game = first_game(&s, &token, &a, T0).await;
    let early = s.start_game(&token, &game, &a, START - HALF - 1).await;
    assert!(matches!(early, Err(StoreError::Conflict("not_yet"))));
    let on_time = s.start_game(&token, &game, &a, START - HALF).await.unwrap();
    assert_eq!(on_time.players.len(), 4);
    assert!(on_time.format.is_none());
    let json = serde_json::to_value(&on_time).unwrap();
    assert_eq!(json["format"], Value::Null);
    assert_eq!(json["from"]["kind"], "game");
    // Until the game's end, then no more.
    s.start_game(&token, &game, &a, START + HOUR - 1)
        .await
        .unwrap();
    let late = s.start_game(&token, &game, &a, START + HOUR).await;
    assert!(matches!(late, Err(StoreError::Conflict("game_over"))));
}

#[tokio::test]
async fn start_creates_auto_linked_entries_for_the_starter() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let game = first_game(&s, &token, &a, T0).await;
    let started = s.start_game(&token, &game, &b, START).await.unwrap();
    let rows: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT name, linked_user_id, link_auto FROM players WHERE owner_user_id = ? ORDER BY name",
    )
    .bind(&b)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4);
    for (who, auto) in [(&a, true), (&b, false), (&c, true), (&d, true)] {
        let row = rows.iter().find(|r| r.1.as_deref() == Some(who)).unwrap();
        assert_eq!(row.2, auto);
    }
    // Seat order, with the entries' names and ids.
    let names: Vec<_> = started.players.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Ana", "Rui", "Marta", "Joana"]);
    // A second look reuses the entries.
    s.start_game(&token, &game, &b, START).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM players WHERE owner_user_id = ?")
        .bind(&b)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 4);
}

#[tokio::test]
async fn non_player_cannot_start() {
    let (s, _pool, token, [a, ..]) = formed().await;
    let game = first_game(&s, &token, &a, T0).await;
    let outsider = user(&s, "Zed").await;
    let res = s.start_game(&token, &game, &outsider, START).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    let res = s.start_game("nope", &game, &a, START).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn a_short_game_starts_with_the_players_it_has() {
    let (s, _pool, token, [a, ..]) = formed().await;
    let game = first_game(&s, &token, &a, T0).await;
    let c = &formed_ids(&s, &token, &game).await[3];
    s.leave_game(
        &token,
        &game,
        c,
        view(&s, &token, &a, T0).await.games[0].version,
        T0 + 5,
    )
    .await
    .unwrap();
    let out = s.start_game(&token, &game, &a, START).await.unwrap();
    assert_eq!(out.players.len(), 3);
}

async fn formed_ids(s: &Store, token: &str, game: &str) -> Vec<String> {
    let v = s.board_view(token, None, T0).await.unwrap().unwrap();
    v.games
        .iter()
        .find(|g| g.id == game)
        .unwrap()
        .players
        .iter()
        .map(|p| p.user_id.clone())
        .collect()
}

fn match_json(from: Value) -> Value {
    let mut body = serde_json::json!({
        "format": Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::One,
            deciding: DecidingSet::FullSet,
        }),
        "start": Start {
            first_server: Player { team: Team::A, index: 0 },
            other_first_server: 0,
        },
        "teams": { "a": ["Ana", "Rui"], "b": ["Marta", "Joana"] },
    });
    body["from"] = from;
    body
}

/// A board and a game that started a few minutes ago on the real clock, with four users who
/// are all in it: (state, token, game id, [(user id, session)]).
async fn live_game() -> (
    padel_score_server::AppState,
    SqlitePool,
    String,
    String,
    Vec<(String, String)>,
) {
    let (state, pool) = support::state().await;
    let real = padel_score_server::api::now();
    let game_start = real / 1800 * 1800;
    let mut users = Vec::new();
    for (i, name) in ["Ana", "Rui", "Marta", "Joana"].into_iter().enumerate() {
        users.push(
            support::fast_user(&state, &format!("{}{i}@x.test", name.to_lowercase()), name).await,
        );
    }
    let token = state
        .store
        .create_board(
            &users[0].0,
            NewBoard {
                title: "Live".to_owned(),
                place: None,
                time_zone: "Europe/Lisbon".to_owned(),
                starts_at: game_start - HOUR,
                ends_at: game_start - HOUR + WEEK_SECS,
            },
            real - 7200,
        )
        .await
        .unwrap()
        .board
        .token;
    for (i, (who, _)) in users.iter().enumerate() {
        state
            .store
            .add_free(
                &token,
                who,
                game_start,
                game_start + HOUR,
                real - 7000 + i64::try_from(i).unwrap(),
            )
            .await
            .unwrap();
    }
    let game = first_game(&state.store, &token, &users[0].0, real - 6000).await;
    (state, pool, token, game, users)
}

#[tokio::test]
async fn match_from_game_marks_it_played() {
    let (state, _pool, token, game, users) = live_game().await;
    let app = support::app_from(&state, 1);
    let raw = &users[0].1;
    let started = support::send(
        &app,
        support::as_session(
            &state,
            raw,
            Method::POST,
            &format!("/api/boards/{token}/games/{game}/start"),
            None,
        ),
    )
    .await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    assert_eq!(started.body["from"]["kind"], "game");
    assert_eq!(started.body["players"].as_array().unwrap().len(), 4);
    let from = started.body["from"].clone();
    let created = support::send(
        &app,
        support::as_session(
            &state,
            raw,
            Method::POST,
            "/api/matches",
            Some(match_json(from)),
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let view = state
        .store
        .board_view(&token, Some(&users[0].0), padel_score_server::api::now())
        .await
        .unwrap()
        .unwrap();
    let g = &view.games[0];
    assert_eq!(g.state, GameState::Played);
    assert_eq!(
        g.match_spectator_token.as_deref(),
        created.body["spectator_token"].as_str()
    );
}

#[tokio::test]
async fn second_start_is_refused() {
    let (state, _pool, token, game, users) = live_game().await;
    let app = support::app_from(&state, 1);
    let from = serde_json::json!({"kind": "game", "id": game});
    let body = || Some(match_json(from.clone()));
    let first = support::send(
        &app,
        support::as_session(&state, &users[0].1, Method::POST, "/api/matches", body()),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    let second = support::send(
        &app,
        support::as_session(&state, &users[1].1, Method::POST, "/api/matches", body()),
    )
    .await;
    assert_eq!(second.status, StatusCode::CONFLICT);
    assert_eq!(second.body["error"], "already_started");
    // The route says the same, and the loser left no match behind.
    let again = support::send(
        &app,
        support::as_session(
            &state,
            &users[1].1,
            Method::POST,
            &format!("/api/boards/{token}/games/{game}/start"),
            None,
        ),
    )
    .await;
    assert_eq!(again.body["error"], "already_started");
}

#[tokio::test]
async fn match_from_game_needs_a_player_and_a_session() {
    let (state, pool, _token, game, _users) = live_game().await;
    let app = support::app_from(&state, 1);
    let from = serde_json::json!({"kind": "game", "id": game});
    let anon = support::send(
        &app,
        support::req(
            Method::POST,
            "/api/matches",
            Some(match_json(from.clone())),
            &[],
        ),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let (_, outsider) = support::fast_user(&state, "zed@x.test", "Zed").await;
    let res = support::send(
        &app,
        support::as_session(
            &state,
            &outsider,
            Method::POST,
            "/api/matches",
            Some(match_json(from)),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // Nothing was created for the refused tries.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
