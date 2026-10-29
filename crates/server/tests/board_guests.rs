#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{
    BoardChange, BoardView, MAX_BOARD_MEMBERS, MAX_PIECES_PER_MEMBER, NewBoard, WEEK_SECS,
};
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

#[tokio::test]
async fn start_game_gives_guests_plain_entries() {
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
    let second_start = START + 2 * HOUR;
    add_free_for(&s, &token, &people, START).await;
    add_free_for(&s, &token, &people, second_start).await;
    let v = view(&s, &token, &a, NOW + 10).await;
    assert_eq!(v.games.len(), 2);

    let mut guest_entries = Vec::new();
    for game in &v.games {
        let started = s
            .start_game(&token, &game.id, &a, game.start)
            .await
            .unwrap();
        assert_eq!(started.players.len(), 4);
        let ze = started.players.iter().find(|p| p.name == "Zé").unwrap();
        guest_entries.push(ze.player_id.clone());
    }
    assert_eq!(guest_entries[0], guest_entries[1]);
    let (owner, linked): (String, Option<String>) =
        sqlx::query_as("SELECT owner_user_id, linked_user_id FROM players WHERE id = ?")
            .bind(&guest_entries[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(owner, a);
    assert_eq!(linked, None);
}

/// The seat the participant holds in the game, if any.
async fn seat_in(pool: &SqlitePool, game: &str, who: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ? AND user_id = ?")
        .bind(game)
        .bind(who)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// The merged `[start, end)` ranges the participant's pieces cover, and whether any two overlap.
fn coverage(v: &BoardView, who: &str) -> (Vec<(i64, i64)>, bool) {
    let mut pieces: Vec<(i64, i64)> = v
        .pieces
        .iter()
        .filter(|p| p.user_id == who)
        .map(|p| (p.start, p.end))
        .collect();
    pieces.sort_unstable();
    let overlapping = pieces.windows(2).any(|w| w[0].1 > w[1].0);
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (s, e) in pieces {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    (merged, overlapping)
}

/// A creator, two members and a guest, all free for the same hour: one game with the guest in it.
async fn game_with_guest() -> (Store, SqlitePool, String, [String; 3], String) {
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
    add_free_for(&s, &token, &people, START).await;
    let v = view(&s, &token, &a, NOW + 10).await;
    assert_eq!(v.games.len(), 1);
    assert!(player_ids(&v).contains(&g));
    (s, pool, token, [a, b, c], g)
}

#[tokio::test]
async fn claim_moves_ranges_and_seat() {
    let (s, pool, token, _, g) = game_with_guest().await;
    let key = s.pick_guest(&token, &g, NOW + 11).await.unwrap().guest_key;
    let d = user(&s, "Joana").await;
    let game = view(&s, &token, &d, NOW + 12).await.games[0].id.clone();
    let seat = seat_in(&pool, &game, &g).await.unwrap();

    let v = s.claim_guest(&token, &d, &g, NOW + 20).await.unwrap();
    assert!(v.members.iter().any(|m| m.user_id == d && !m.guest));
    assert!(v.members.iter().all(|m| m.user_id != g));
    assert_eq!(coverage(&v, &d), (vec![(START, START + HOUR)], false));
    assert!(v.pieces.iter().all(|p| p.user_id != g));
    assert_eq!(seat_in(&pool, &game, &d).await, Some(seat));
    assert_eq!(seat_in(&pool, &game, &g).await, None);
    assert!(v.games[0].players.iter().any(|p| p.user_id == d));
    assert_eq!(s.guest_for_key(&token, &key).await.unwrap(), None);
    assert_eq!(v.me.unwrap().user_id, d);
}

#[tokio::test]
async fn claim_merges_overlapping_free_time() {
    let (s, _, token) = open_board().await;
    let b = user(&s, "Rui").await;
    let (g, _) = guest(&s, &token, "Zé").await;
    s.add_free(&token, &b, START + HOUR, START + 3 * HOUR, NOW)
        .await
        .unwrap();
    s.add_free(&token, &g, START, START + 2 * HOUR, NOW + 1)
        .await
        .unwrap();
    let v = s.claim_guest(&token, &b, &g, NOW + 2).await.unwrap();
    assert_eq!(coverage(&v, &b), (vec![(START, START + 3 * HOUR)], false));
    // The guest's earlier time keeps its place in line.
    let early = v
        .pieces
        .iter()
        .find(|p| p.user_id == b && p.start == START)
        .unwrap();
    assert_eq!(early.created_at, NOW + 1);
}

#[tokio::test]
async fn claim_into_a_game_the_claimer_is_in() {
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
    assert_eq!(before.games[0].line, vec![e.clone()]);
    let game = before.games[0].id.clone();
    let seat = seat_in(&pool, &game, &b).await.unwrap();
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let after = s.claim_guest(&token, &b, &g, NOW + 20).await.unwrap();
    let players = player_ids(&after);
    assert_eq!(players.iter().filter(|p| **p == b).count(), 1);
    assert!(!players.contains(&g));
    assert!(players.contains(&e), "the line fills the freed seat");
    assert_eq!(seat_in(&pool, &game, &b).await, Some(seat));
    assert!(after.games[0].line.is_empty());
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, payload FROM push_outbox ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let left: Vec<&(String, String)> = rows
        .iter()
        .filter(|(_, p)| p.contains("player_left"))
        .collect();
    assert!(!left.is_empty());
    assert!(left.iter().all(|(_, p)| p.contains("Zé")));
    assert!(left.iter().all(|(u, _)| u != &g));
}

#[tokio::test]
async fn claim_frees_the_guest_seat_when_the_claimer_is_busy() {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let c = user(&s, "Marta").await;
    let d = user(&s, "Joana").await;
    let e = user(&s, "Tomás").await;
    let f = user(&s, "Inês").await;
    let h = user(&s, "Luís").await;
    let token = board(&s, &a, true).await;
    for u in [&b, &c, &d, &e, &f, &h] {
        s.join_board(&token, u, NOW).await.unwrap();
    }
    let (g, _) = guest(&s, &token, "Zé").await;
    // Game A for a, b, c, d; game B half an hour later for e, f, h and the guest.
    add_free_for(&s, &token, &[&a, &b, &c, &d], START).await;
    add_free_for(&s, &token, &[&e, &f, &h, &g], START + HOUR / 2).await;
    let before = view(&s, &token, &a, NOW + 10).await;
    assert_eq!(before.games.len(), 2);
    let in_a = before
        .games
        .iter()
        .find(|x| x.players.iter().any(|p| p.user_id == a))
        .unwrap()
        .id
        .clone();
    let in_b = before
        .games
        .iter()
        .find(|x| x.players.iter().any(|p| p.user_id == g))
        .unwrap()
        .id
        .clone();
    assert_ne!(in_a, in_b);

    let v = s.claim_guest(&token, &a, &g, NOW + 20).await.unwrap();
    assert!(seat_in(&pool, &in_a, &a).await.is_some());
    assert_eq!(seat_in(&pool, &in_b, &a).await, None);
    assert_eq!(seat_in(&pool, &in_b, &g).await, None);
    let mine: Vec<_> = v
        .games
        .iter()
        .filter(|x| x.players.iter().any(|p| p.user_id == a))
        .collect();
    assert_eq!(mine.len(), 1);
    for x in &v.games {
        assert!(x.players.iter().filter(|p| p.user_id == a).count() <= 1);
    }
    for (i, x) in mine.iter().enumerate() {
        for y in mine.iter().skip(i + 1) {
            assert!(x.end <= y.start || y.end <= x.start, "overlapping games");
        }
    }
}

#[tokio::test]
async fn claim_leaves_played_seats_with_the_guest() {
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
    add_free_for(&s, &token, &people, START).await;
    let v = view(&s, &token, &a, NOW + 10).await;
    let from = StartedFrom::Game {
        id: v.games[0].id.clone(),
    };
    create_match_from(&s, new_match(&a), &from, START)
        .await
        .unwrap();
    let d = user(&s, "Joana").await;
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let after = s.claim_guest(&token, &d, &g, START + 600).await.unwrap();
    let game = &after.games[0];
    assert!(game.players.iter().any(|p| p.user_id == g && p.guest));
    assert!(game.players.iter().any(|p| p.name == "Zé"));
    assert!(game.players.iter().all(|p| p.user_id != d));
    assert_eq!(seat_in(&pool, &game.id, &d).await, None);
    let queued: Vec<String> = sqlx::query_scalar("SELECT payload FROM push_outbox")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        queued.iter().all(|p| !p.contains("player_left")),
        "unexpected player_left: {queued:?}"
    );
    assert!(after.members.iter().any(|m| m.user_id == d));
}

#[tokio::test]
async fn claim_expires_the_guests_requests() {
    let (s, pool, token, [a, b, _], g) = game_with_guest().await;
    let v = view(&s, &token, &a, NOW + 10).await;
    let game = v.games[0].id.clone();
    for (from, to) in [(&g, &b), (&b, &g)] {
        sqlx::query(
            "INSERT INTO board_requests (id, board_id, kind, from_user_id, game_id, game_version, \
             to_user_id, created_at) \
             SELECT ?, board_id, 'swap', ?, id, version, ?, ? FROM board_games WHERE id = ?",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(from)
        .bind(to)
        .bind(NOW)
        .bind(&game)
        .execute(&pool)
        .await
        .unwrap();
    }
    let d = user(&s, "Joana").await;
    s.claim_guest(&token, &d, &g, NOW + 30).await.unwrap();
    let rows: Vec<(String, Option<i64>)> =
        sqlx::query_as("SELECT status, decided_at FROM board_requests")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|(st, at)| st == "expired" && *at == Some(NOW + 30))
    );
}

#[tokio::test]
async fn claim_needs_a_live_guest_of_this_board() {
    let (s, _, token) = open_board().await;
    let d = user(&s, "Joana").await;
    let (gone, _) = guest(&s, &token, "Gone").await;
    s.leave_board(&token, &gone, NOW + 1).await.unwrap();
    let (live, _) = guest(&s, &token, "Zé").await;
    let c = user(&s, "Marta").await;
    let other = board(&s, &c, true).await;
    let (foreign, _) = guest(&s, &other, "Foreign").await;
    let ana = creator_of(&s, &token).await;

    for id in [&gone, &foreign, &ana, &"guest_nope".to_owned()] {
        assert!(matches!(
            s.claim_guest(&token, &d, id, NOW + 2).await,
            Err(StoreError::NotFound)
        ));
    }
    // Only accounts claim: a guest id as the claimer.
    assert!(matches!(
        s.claim_guest(&token, &live, &live, NOW + 2).await,
        Err(StoreError::NotFound)
    ));
    let v = view(&s, &token, &d, NOW + 3).await;
    assert!(v.members.iter().all(|m| m.user_id != d));
    assert!(v.members.iter().any(|m| m.user_id == live));
}

#[tokio::test]
async fn claim_respects_the_piece_cap() {
    let (s, _, token) = open_board().await;
    let a = creator_of(&s, &token).await;
    let (g, key) = guest(&s, &token, "Zé").await;
    let cap = i64::try_from(MAX_PIECES_PER_MEMBER).unwrap();
    // `cap` single-slot pieces with one slot missing at index `cap - 1`.
    for i in (0..cap - 1).chain([cap]) {
        let at = START + i * 1800;
        s.add_free(&token, &a, at, at + 1800, NOW).await.unwrap();
    }
    let gap = START + (cap - 1) * 1800;
    s.add_free(&token, &g, gap, gap + 1800, NOW + 1)
        .await
        .unwrap();
    conflict(
        s.claim_guest(&token, &a, &g, NOW + 2).await,
        "too_many_ranges",
    );
    let v = view(&s, &token, &a, NOW + 3).await;
    assert_eq!(
        v.pieces.iter().filter(|p| p.user_id == a).count(),
        MAX_PIECES_PER_MEMBER
    );
    assert!(v.members.iter().any(|m| m.user_id == g));
    assert!(v.pieces.iter().any(|p| p.user_id == g));
    assert_eq!(s.guest_for_key(&token, &key).await.unwrap(), Some(g));
}
