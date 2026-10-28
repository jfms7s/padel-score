#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{
    BoardChange, BoardView, GameState, MAX_BOARD_MEMBERS, MAX_PIECES_PER_MEMBER, NewBoard,
    WEEK_SECS,
};
use padel_score_server::db;
use padel_score_server::store::{Store, StoreError};
use serde_json::{Value, json};
use sqlx::SqlitePool;

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
const HOUR: i64 = 3600;
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

fn new_board(starts_at: i64) -> NewBoard {
    NewBoard {
        title: "Thursday padel".to_owned(),
        place: Some("Club".to_owned()),
        time_zone: "Europe/Lisbon".to_owned(),
        starts_at,
        ends_at: starts_at + WEEK_SECS,
    }
}

/// A board created by `creator` at `T0`: (token, view).
async fn board(s: &Store, creator: &str) -> String {
    s.create_board(creator, new_board(START), T0)
        .await
        .unwrap()
        .board
        .token
}

/// Four users and a board where all four have added `[START, START + 1h)`, each a second
/// apart, so a game has formed with them in that order.
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

fn kinds(rows: &[(String, Value)], kind: &str) -> Vec<String> {
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

fn player_ids(view: &BoardView, game: usize) -> Vec<String> {
    view.games[game]
        .players
        .iter()
        .map(|p| p.user_id.clone())
        .collect()
}

#[tokio::test]
async fn create_board_makes_creator_a_member() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let view = s.create_board(&a, new_board(START), T0).await.unwrap();
    assert_eq!(view.board.title, "Thursday padel");
    assert_eq!(view.board.place.as_deref(), Some("Club"));
    assert_eq!(view.board.creator_user_id, a);
    assert_eq!(view.board.creator_name, "Ana");
    assert!(!view.board.over);
    assert_eq!(view.members.len(), 1);
    assert_eq!(view.members[0].user_id, a);
    assert_eq!(view.members[0].joined_at, T0);
    let me = view.me.unwrap();
    assert!(me.member && me.creator && me.user_id == a);
    assert!(view.games.is_empty() && view.pieces.is_empty() && view.requests.is_empty());
    assert!(!view.board.token.is_empty());
}

#[tokio::test]
async fn board_week_may_be_seven_days_plus_or_minus_an_hour() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    for slack in [3600, -3600, 0] {
        let mut b = new_board(START);
        b.ends_at = START + WEEK_SECS + slack;
        s.create_board(&a, b, T0).await.unwrap();
    }
    for slack in [3601, -3601] {
        let mut b = new_board(START);
        b.ends_at = START + WEEK_SECS + slack;
        let err = s.create_board(&a, b, T0).await.unwrap_err();
        assert!(matches!(err, StoreError::InvalidField("week")), "{err:?}");
    }
    // A week that is already over.
    let err = s
        .create_board(&a, new_board(T0 - WEEK_SECS - 100), T0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("week")), "{err:?}");
}

#[tokio::test]
async fn bad_title_place_or_time_zone_is_refused() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let try_with = |f: &dyn Fn(&mut NewBoard)| {
        let mut b = new_board(START);
        f(&mut b);
        b
    };
    for title in ["", "   ", &"x".repeat(41)] {
        let err = s
            .create_board(&a, try_with(&|b| b.title = title.to_owned()), T0)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidField("title")), "{err:?}");
    }
    let err = s
        .create_board(&a, try_with(&|b| b.place = Some("p".repeat(81))), T0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("place")), "{err:?}");
    for tz in [
        "",
        &"Z".repeat(65),
        "Europe/Lis bon",
        "Europe/Lisbon;",
        "Zone\u{e9}",
    ] {
        let err = s
            .create_board(&a, try_with(&|b| b.time_zone = tz.to_owned()), T0)
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("time_zone")),
            "{err:?}"
        );
    }
    // Trimmed title, blank place means none, a 64-char zone and "Etc/GMT+1" pass.
    let view = s
        .create_board(
            &a,
            try_with(&|b| {
                b.title = "  Club night  ".to_owned();
                b.place = Some("  ".to_owned());
                b.time_zone = "Etc/GMT+1".to_owned();
            }),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(view.board.title, "Club night");
    assert_eq!(view.board.place, None);
    s.create_board(&a, try_with(&|b| b.time_zone = "z".repeat(64)), T0)
        .await
        .unwrap();
}

#[tokio::test]
async fn the_twenty_first_open_board_is_refused_but_over_ones_do_not_count() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    for _ in 0..20 {
        s.create_board(&a, new_board(START), T0).await.unwrap();
    }
    let err = s.create_board(&a, new_board(START), T0).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_many_boards")),
        "{err:?}"
    );
    // Once those are over, a new one is fine.
    let later = START + WEEK_SECS;
    s.create_board(&a, new_board(later + DAY), later)
        .await
        .unwrap();
}

#[tokio::test]
async fn range_must_sit_on_the_half_hour_grid_inside_the_week() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    let bad = [
        (START + 1, START + HOUR),
        (START, START + HOUR + 1),
        (START + 1800, START + 1800),
        (START + HOUR, START),
        (START - 1800, START + HOUR),
        (START, START + WEEK_SECS + 1800),
    ];
    for (start, end) in bad {
        let err = s.add_free(&token, &a, start, end, T0).await.unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("range")),
            "{start} {end} {err:?}"
        );
        let err = s.remove_free(&token, &a, start, end, T0).await.unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("range")),
            "{start} {end} {err:?}"
        );
    }
    let view = s
        .add_free(&token, &a, START, START + WEEK_SECS, T0)
        .await
        .unwrap();
    assert_eq!(view.pieces.len(), 1);
    assert_eq!(view.pieces[0].start, START);
    assert_eq!(view.pieces[0].end, START + WEEK_SECS);
    assert_eq!(view.pieces[0].created_at, T0);
}

#[tokio::test]
async fn adding_covered_time_adds_nothing_and_a_member_may_hold_200_pieces() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    s.add_free(&token, &a, START, START + HOUR, T0)
        .await
        .unwrap();
    let view = s
        .add_free(&token, &a, START, START + HOUR, T0 + 5)
        .await
        .unwrap();
    assert_eq!(view.pieces.len(), 1);
    assert_eq!(view.pieces[0].created_at, T0);
    // Growing a range adds a piece for the new part only.
    let view = s
        .add_free(&token, &a, START, START + 2 * HOUR, T0 + 6)
        .await
        .unwrap();
    assert_eq!(view.pieces.len(), 2);
    assert_eq!(view.pieces[1].start, START + HOUR);
    assert_eq!(view.pieces[1].created_at, T0 + 6);
    // Fill up to 200 pieces, one slot each, then the next is refused.
    let mut n = i64::try_from(view.pieces.len()).unwrap();
    while n < i64::try_from(MAX_PIECES_PER_MEMBER).unwrap() {
        let at = START + 2 * HOUR + (n - 2) * 1800;
        s.add_free(&token, &a, at, at + 1800, T0 + 10)
            .await
            .unwrap();
        n += 1;
    }
    let at = START + 2 * HOUR + (n - 2) * 1800;
    let err = s
        .add_free(&token, &a, at, at + 1800, T0 + 10)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_many_ranges")),
        "{err:?}"
    );
}

#[tokio::test]
async fn add_free_joins_the_board() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let token = board(&s, &a).await;
    let view = s
        .add_free(&token, &b, START, START + HOUR, T0 + 3)
        .await
        .unwrap();
    assert_eq!(view.members.len(), 2);
    assert_eq!(view.members[1].user_id, b);
    assert_eq!(view.members[1].joined_at, T0 + 3);
    let me = view.me.unwrap();
    assert!(me.member && !me.creator);
}

#[tokio::test]
async fn four_overlapping_ranges_form_a_game_and_queue_four_notices() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let view = s
        .board_view(&token, Some(&a), T0 + 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.games.len(), 1);
    let game = &view.games[0];
    assert_eq!((game.start, game.end), (START, START + HOUR));
    assert_eq!(game.state, GameState::Full);
    assert_eq!(game.version, 0);
    assert_eq!(
        player_ids(&view, 0),
        [a.clone(), b.clone(), c.clone(), d.clone()]
    );
    assert!(game.line.is_empty());
    assert_eq!(game.match_spectator_token, None);

    let rows = outbox(&pool).await;
    assert_eq!(rows.len(), 4);
    assert_eq!(kinds(&rows, "game_formed"), sorted(&[&a, &b, &c, &d]));
    for (_, p) in &rows {
        assert_eq!(p["url"], format!("/b/{token}"));
        assert_eq!(p["title"], "Thursday padel");
        assert_eq!(p["start"], START);
        assert_eq!(p["end"], START + HOUR);
    }
}

#[tokio::test]
async fn a_game_stretches_to_ninety_minutes_when_all_four_are_free() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    let mut last = None;
    for (i, who) in [
        a,
        user(&s, "Rui").await,
        user(&s, "Marta").await,
        user(&s, "Joana").await,
    ]
    .iter()
    .enumerate()
    {
        let now = T0 + i64::try_from(i).unwrap();
        last = Some(
            s.add_free(&token, who, START, START + 5400, now)
                .await
                .unwrap(),
        );
    }
    let view = last.unwrap();
    assert_eq!(view.games.len(), 1);
    assert_eq!(view.games[0].end, START + 5400);
}

#[tokio::test]
async fn concurrent_adds_form_one_game() {
    let path = std::env::temp_dir().join(format!("padel-{}.db", uuid::Uuid::new_v4()));
    let pool = db::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    let s = Store::new(pool.clone());
    let mut users = Vec::new();
    for name in ["Ana", "Rui", "Marta", "Joana", "Tiago"] {
        users.push(user(&s, name).await);
    }
    let token = board(&s, &users[0]).await;
    for (i, who) in users[..3].iter().enumerate() {
        let now = T0 + i64::try_from(i).unwrap();
        s.add_free(&token, who, START, START + HOUR, now)
            .await
            .unwrap();
    }
    let (s1, s2) = (s.clone(), s.clone());
    let (t1, t2) = (token.clone(), token.clone());
    let (u4, u5) = (users[3].clone(), users[4].clone());
    let (r4, r5) = tokio::join!(
        async move { s1.add_free(&t1, &u4, START, START + HOUR, T0 + 3).await },
        async move { s2.add_free(&t2, &u5, START, START + HOUR, T0 + 4).await },
    );
    r4.unwrap();
    r5.unwrap();

    let games: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_games")
        .fetch_one(&pool)
        .await
        .unwrap();
    let seats: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_game_players")
        .fetch_one(&pool)
        .await
        .unwrap();
    let distinct: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT user_id) FROM board_game_players")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((games, seats, distinct), (1, 4, 4));
    let formed = outbox(&pool)
        .await
        .iter()
        .filter(|(_, p)| p["kind"] == "game_formed")
        .count();
    assert_eq!(formed, 4);

    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn trimming_a_range_under_a_game_leaves_it_and_the_line_fills() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Tiago").await;
    // The fifth person is free for the game but formed later, so they are in the line.
    let view = s
        .add_free(&token, &e, START, START + HOUR, T0 + 50)
        .await
        .unwrap();
    assert_eq!(view.games[0].line, std::slice::from_ref(&e));
    assert_eq!(view.games[0].version, 0);
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();

    let view = s
        .remove_free(&token, &a, START + 1800, START + HOUR, T0 + 60)
        .await
        .unwrap();
    // Ana keeps the half hour she did not remove, but is out of the game; Tiago has her seat.
    assert_eq!(view.pieces.iter().filter(|p| p.user_id == a).count(), 1);
    assert_eq!(
        player_ids(&view, 0),
        [e.clone(), b.clone(), c.clone(), d.clone()]
    );
    assert_eq!(view.games[0].state, GameState::Full);
    assert_eq!(view.games[0].version, 2);

    let rows = outbox(&pool).await;
    assert_eq!(kinds(&rows, "player_left"), sorted(&[&b, &c, &d]));
    assert_eq!(kinds(&rows, "moved_in"), vec![e.clone()]);
    let left = rows
        .iter()
        .find(|(_, p)| p["kind"] == "player_left")
        .unwrap();
    assert_eq!(left.1["name"], "Ana");
    assert_eq!(left.1["url"], format!("/b/{token}"));
    assert_eq!(left.1["start"], START);
}

#[tokio::test]
async fn a_short_game_with_nobody_in_line_stays_short() {
    let (s, _, token, [a, b, c, d]) = formed().await;
    let view = s
        .remove_free(&token, &d, START, START + HOUR, T0 + 60)
        .await
        .unwrap();
    assert_eq!(player_ids(&view, 0), [a, b, c]);
    assert_eq!(view.games[0].state, GameState::Short);
}

#[tokio::test]
async fn leaving_the_board_removes_ranges_and_games() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Tiago").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 50)
        .await
        .unwrap();
    s.leave_board(&token, &b, T0 + 60).await.unwrap();

    let view = s
        .board_view(&token, Some(&a), T0 + 61)
        .await
        .unwrap()
        .unwrap();
    assert!(view.members.iter().all(|m| m.user_id != b));
    assert!(view.pieces.iter().all(|p| p.user_id != b));
    // The line filled the seat.
    assert_eq!(
        player_ids(&view, 0),
        [a.clone(), e.clone(), c.clone(), d.clone()]
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_game_players WHERE user_id = ?")
        .bind(&b)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    // Leaving again is a no-op.
    s.leave_board(&token, &b, T0 + 62).await.unwrap();

    // When everyone has left, the game is gone with them.
    for who in [&a, &e, &c, &d] {
        s.leave_board(&token, who, T0 + 70).await.unwrap();
    }
    let games: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_games")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(games, 0);
}

#[tokio::test]
async fn adding_time_never_reshuffles_a_formed_game() {
    let (s, _, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Tiago").await;
    let f = user(&s, "Sofia").await;
    // Both added later, and one claims to have added before everyone.
    s.add_free(&token, &e, START, START + HOUR, T0 + 100)
        .await
        .unwrap();
    let view = s
        .add_free(&token, &f, START, START + HOUR, T0 - 100)
        .await
        .unwrap();
    assert_eq!(view.games.len(), 1);
    assert_eq!(player_ids(&view, 0), [a, b, c, d]);
    assert_eq!(view.games[0].version, 0);
    assert_eq!(view.games[0].line, [f, e]);
}

#[tokio::test]
async fn over_board_is_read_only() {
    let (s, _, token, [a, b, ..]) = formed().await;
    let over = START + WEEK_SECS;
    let stranger = user(&s, "Tiago").await;
    for err in [
        s.add_free(&token, &a, START, START + HOUR, over)
            .await
            .unwrap_err(),
        s.remove_free(&token, &a, START, START + HOUR, over)
            .await
            .unwrap_err(),
        s.join_board(&token, &stranger, over).await.unwrap_err(),
        s.leave_board(&token, &b, over).await.unwrap_err(),
        s.update_board(&token, &a, BoardChange::default(), over)
            .await
            .unwrap_err(),
        s.delete_board(&token, &a, over).await.unwrap_err(),
    ] {
        assert!(matches!(err, StoreError::Conflict("board_over")), "{err:?}");
    }
    let view = s.board_view(&token, Some(&a), over).await.unwrap().unwrap();
    assert!(view.board.over);
    assert_eq!(view.games.len(), 1);
    assert_eq!(view.games[0].state, GameState::Past);
}

#[tokio::test]
async fn unknown_token_is_not_found_for_writes_and_none_for_the_view() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    assert!(s.board_view("nope", Some(&a), T0).await.unwrap().is_none());
    assert!(matches!(
        s.join_board("nope", &a, T0).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.add_free("nope", &a, START, START + HOUR, T0).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.leave_board("nope", &a, T0).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.delete_board("nope", &a, T0).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn only_the_creator_edits_or_deletes() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let token = board(&s, &a).await;
    s.join_board(&token, &b, T0).await.unwrap();
    let change = || BoardChange {
        title: Some("New name".to_owned()),
        place: None,
    };
    let err = s.update_board(&token, &b, change(), T0).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("creator_only")),
        "{err:?}"
    );
    let err = s.delete_board(&token, &b, T0).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("creator_only")),
        "{err:?}"
    );

    let view = s.update_board(&token, &a, change(), T0).await.unwrap();
    assert_eq!(view.board.title, "New name");
    assert_eq!(view.board.place.as_deref(), Some("Club"));
    let view = s
        .update_board(
            &token,
            &a,
            BoardChange {
                title: None,
                place: Some(None),
            },
            T0,
        )
        .await
        .unwrap();
    assert_eq!(view.board.place, None);
    assert_eq!(view.board.title, "New name");
    let err = s
        .update_board(
            &token,
            &a,
            BoardChange {
                title: Some(" ".to_owned()),
                place: None,
            },
            T0,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("title")), "{err:?}");

    s.delete_board(&token, &a, T0).await.unwrap();
    assert!(s.board_view(&token, Some(&a), T0).await.unwrap().is_none());
}

#[tokio::test]
async fn board_change_tells_an_absent_place_from_a_null_one() {
    let absent: BoardChange = serde_json::from_value(json!({"title": "x"})).unwrap();
    assert_eq!(absent.place, None);
    let null: BoardChange = serde_json::from_value(json!({"place": null})).unwrap();
    assert_eq!(null.place, Some(None));
    let set: BoardChange = serde_json::from_value(json!({"place": "Club"})).unwrap();
    assert_eq!(set.place, Some(Some("Club".to_owned())));
}

#[tokio::test]
async fn delete_board_tells_players_with_games() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let watcher = user(&s, "Tiago").await;
    s.join_board(&token, &watcher, T0 + 5).await.unwrap();
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();
    s.delete_board(&token, &a, T0 + 10).await.unwrap();
    let rows = outbox(&pool).await;
    // The deleter is not told; the member without a game is not either.
    assert_eq!(kinds(&rows, "board_deleted"), sorted(&[&b, &c, &d]));
    assert_eq!(rows.len(), 3);
    for (_, p) in &rows {
        assert_eq!(p["url"], "/calendar");
        assert_eq!(p["title"], "Thursday padel");
        assert_eq!(p["start"], START);
        assert_eq!(p["end"], START + HOUR);
    }
    let gone: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_game_players")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(gone, 0);
}

#[tokio::test]
async fn delete_board_does_not_tell_players_of_past_games() {
    let (s, pool, token, [a, ..]) = formed().await;
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();
    s.delete_board(&token, &a, START + HOUR).await.unwrap();
    assert!(outbox(&pool).await.is_empty());
}

#[tokio::test]
async fn signed_out_view_has_no_me() {
    let (s, _, token, [a, ..]) = formed().await;
    let stranger = user(&s, "Tiago").await;
    let out = s.board_view(&token, None, T0 + 10).await.unwrap().unwrap();
    assert!(out.me.is_none());
    assert_eq!(out.members.len(), 4);
    assert_eq!(out.games.len(), 1);
    let other = s
        .board_view(&token, Some(&stranger), T0 + 10)
        .await
        .unwrap()
        .unwrap();
    let me = other.me.unwrap();
    assert!(!me.member && !me.creator && me.user_id == stranger);
    let creator = s
        .board_view(&token, Some(&a), T0 + 10)
        .await
        .unwrap()
        .unwrap();
    assert!(creator.me.unwrap().creator);
}

#[tokio::test]
async fn viewing_never_forms_a_game() {
    let (s, pool, token, [a, ..]) = formed().await;
    sqlx::query("DELETE FROM board_games")
        .execute(&pool)
        .await
        .unwrap();
    let view = s
        .board_view(&token, Some(&a), T0 + 10)
        .await
        .unwrap()
        .unwrap();
    assert!(view.games.is_empty());
}

#[tokio::test]
async fn join_is_idempotent_and_the_board_holds_64() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    let first = s.join_board(&token, &a, T0 + 1).await.unwrap();
    assert_eq!(first.members.len(), 1);
    assert_eq!(first.members[0].joined_at, T0);
    for i in 1..MAX_BOARD_MEMBERS {
        let u = user(&s, &format!("P{i}")).await;
        s.join_board(&token, &u, T0 + 1).await.unwrap();
    }
    let late = user(&s, "Late").await;
    let err = s.join_board(&token, &late, T0 + 2).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict("board_full")), "{err:?}");
    let err = s
        .add_free(&token, &late, START, START + HOUR, T0 + 2)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Conflict("board_full")), "{err:?}");
    // A member can still join again when it is full.
    let view = s.join_board(&token, &a, T0 + 3).await.unwrap();
    assert_eq!(view.members.len(), MAX_BOARD_MEMBERS);
}

#[tokio::test]
async fn my_boards_lists_open_boards_the_user_is_in() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let mine = board(&s, &a).await;
    let theirs = board(&s, &b).await;
    let old = s
        .create_board(&a, new_board(T0 - WEEK_SECS + DAY), T0 - 2 * DAY)
        .await
        .unwrap();
    s.join_board(&theirs, &a, T0).await.unwrap();
    let boards = s.my_boards(&a, T0).await.unwrap();
    let tokens: Vec<&str> = boards.iter().map(|b| b.token.as_str()).collect();
    assert_eq!(boards.len(), 3);
    assert!(tokens.contains(&mine.as_str()) && tokens.contains(&theirs.as_str()));
    // The old one ends a day ago relative to a later clock.
    let later = s.my_boards(&a, old.board.ends_at).await.unwrap();
    assert_eq!(later.len(), 2);
    assert!(s.my_boards(&b, T0).await.unwrap().len() == 1);
}

#[tokio::test]
async fn the_player_whose_tap_fills_a_seat_is_not_told_but_the_game_is_unchanged_for_others() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    s.leave_board(&token, &d, T0 + 20).await.unwrap();
    let e = user(&s, "Tiago").await;
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();
    let view = s
        .add_free(&token, &e, START, START + HOUR, T0 + 30)
        .await
        .unwrap();
    assert_eq!(player_ids(&view, 0), [a, b, c, e]);
    assert!(outbox(&pool).await.is_empty());
}

#[tokio::test]
async fn removing_time_inside_the_fill_cutoff_leaves_the_game_short() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    let e = user(&s, "Tiago").await;
    s.add_free(&token, &e, START, START + HOUR, T0 + 50)
        .await
        .unwrap();
    sqlx::query("DELETE FROM push_outbox")
        .execute(&pool)
        .await
        .unwrap();
    // 30 minutes before the start: the leaver goes, the line is not used any more.
    let view = s
        .remove_free(&token, &a, START, START + HOUR, START - 1800)
        .await
        .unwrap();
    assert_eq!(player_ids(&view, 0), [b.clone(), c.clone(), d.clone()]);
    assert_eq!(view.games[0].state, GameState::Short);
    let rows = outbox(&pool).await;
    assert!(kinds(&rows, "moved_in").is_empty());
    assert_eq!(kinds(&rows, "player_left"), sorted(&[&b, &c, &d]));
}

#[tokio::test]
async fn cutting_the_middle_of_a_piece_counts_against_the_200_cap() {
    let (s, _) = setup().await;
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    s.add_free(&token, &a, START, START + 4 * 1800, T0)
        .await
        .unwrap();
    for i in 0..(MAX_PIECES_PER_MEMBER - 1) {
        let at = START + (10 + i64::try_from(i).unwrap()) * 1800;
        s.add_free(&token, &a, at, at + 1800, T0).await.unwrap();
    }
    let err = s
        .remove_free(&token, &a, START + 1800, START + 3600, T0)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_many_ranges")),
        "{err:?}"
    );
    // The refused cut changed nothing.
    let view = s.board_view(&token, Some(&a), T0).await.unwrap().unwrap();
    assert_eq!(view.pieces.len(), MAX_PIECES_PER_MEMBER);
    // Cutting an end does not add a piece.
    s.remove_free(&token, &a, START + 3 * 1800, START + 4 * 1800, T0)
        .await
        .unwrap();
}

#[tokio::test]
async fn leaving_takes_the_member_out_of_a_game_that_has_started() {
    let (s, pool, token, [a, b, c, d]) = formed().await;
    s.leave_board(&token, &b, START + 600).await.unwrap();
    let view = s
        .board_view(&token, Some(&a), START + 600)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(player_ids(&view, 0), [a, c, d]);
    let seats: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM board_game_players WHERE user_id = ?")
            .bind(&b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(seats, 0);
}

#[tokio::test]
async fn the_creator_may_leave_and_still_edit_and_delete() {
    let (s, _, token, [a, ..]) = formed().await;
    s.leave_board(&token, &a, T0 + 20).await.unwrap();
    let view = s
        .update_board(
            &token,
            &a,
            BoardChange {
                title: Some("Renamed".to_owned()),
                place: None,
            },
            T0 + 21,
        )
        .await
        .unwrap();
    assert_eq!(view.board.title, "Renamed");
    let me = view.me.unwrap();
    assert!(!me.member && me.creator);
    s.delete_board(&token, &a, T0 + 22).await.unwrap();
    assert!(
        s.board_view(&token, Some(&a), T0 + 23)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_view_is_one_snapshot_and_does_not_wait_for_a_writer() {
    let path = std::env::temp_dir().join(format!("padel-{}.db", uuid::Uuid::new_v4()));
    let pool = db::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    let s = Store::new(pool.clone());
    let a = user(&s, "Ana").await;
    let token = board(&s, &a).await;
    let mut tx = s.begin().await.unwrap();
    sqlx::query("UPDATE boards SET title = 'Changed'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let before = s.board_view(&token, Some(&a), T0).await.unwrap().unwrap();
    assert_eq!(before.board.title, "Thursday padel");
    tx.commit().await.unwrap();
    let after = s.board_view(&token, Some(&a), T0).await.unwrap().unwrap();
    assert_eq!(after.board.title, "Changed");

    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
