//! The calendar: one list over my games, events and matches, what waits for my answer, and what
//! is open to join (Task 18). Store tests use an explicit clock from `T0`; the route test the
//! real one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::http::{Method, StatusCode, header};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::api::now;
use padel_score_server::boards::{NewBoard, WEEK_SECS};
use padel_score_server::competitions::schedule::{ScheduleInput, Signups};
use padel_score_server::competitions::signups::JoinAs;
use padel_score_server::competitions::{Kind, NewCompetition};
use padel_score_server::db;
use padel_score_server::scheduled::{Listing, NewScheduled};
use padel_score_server::store::Store;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{app_from, as_session, fast_user, send, state};

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
const HOUR: i64 = 3600;
const START: i64 = T0 + DAY;

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

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
            allow_guests: false,
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

/// Four users who all added `[start, start + 1h)`: a game formed. (store, token, [a, b, c, d])
async fn formed(start: i64) -> (Store, SqlitePool, String, [String; 4]) {
    let (s, pool) = setup().await;
    let a = user(&s, "Ana").await;
    let b = user(&s, "Rui").await;
    let c = user(&s, "Marta").await;
    let d = user(&s, "Joana").await;
    let token = board(&s, &a).await;
    for (i, who) in [&a, &b, &c, &d].into_iter().enumerate() {
        let now = T0 + i64::try_from(i).unwrap();
        s.add_free(&token, who, start, start + HOUR, now)
            .await
            .unwrap();
    }
    (s, pool, token, [a, b, c, d])
}

/// Two overlapping full games: the first starts at `START`, the second half an hour later.
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
        s.add_free(&token, who, START + HOUR / 2, START + HOUR / 2 + HOUR, now)
            .await
            .unwrap();
    }
    // `first[0]` is also free for the second game and `second[0]` for the first: they can swap.
    s.add_free(
        &token,
        &first[0],
        START + HOUR,
        START + HOUR + HOUR / 2,
        T0 + 20,
    )
    .await
    .unwrap();
    s.add_free(&token, &second[0], START, START + HOUR / 2, T0 + 21)
        .await
        .unwrap();
    (
        s,
        pool,
        token,
        first.try_into().unwrap(),
        second.try_into().unwrap(),
    )
}

/// A tournament starting at `start` with sign-ups `signups`, made at `T0`: (event id, join token).
async fn event(
    s: &Store,
    owner: &str,
    name: &str,
    start: i64,
    signups: Signups,
) -> (String, String) {
    let ev = s
        .create_competition(
            owner,
            NewCompetition {
                name: name.into(),
                kind: Kind::Tournament,
                format: one_set(),
                settings: None,
                schedule: Some(ScheduleInput {
                    starts_at: Some(Some(start)),
                    place: Some(Some("Club".to_owned())),
                    signups: Some(signups),
                    max_teams: Some(Some(8)),
                    ..Default::default()
                }),
            },
            T0,
        )
        .await
        .unwrap();
    (ev.id, ev.join_token.unwrap())
}

async fn scheduled(s: &Store, owner: &str, start: i64, listing: Listing) -> (String, String) {
    let v = s
        .create_scheduled(
            owner,
            NewScheduled {
                starts_at: start,
                place: Some("Court 2".to_owned()),
                listing,
                format: one_set(),
                places: Default::default(),
            },
            T0,
        )
        .await
        .unwrap();
    (v.id, v.join_token)
}

async fn cal(s: &Store, who: &str, from: i64, at: i64) -> Value {
    serde_json::to_value(s.calendar(who, from, at).await.unwrap()).unwrap()
}

fn kinds(items: &Value) -> Vec<String> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn calendar_lists_my_games_events_and_matches_by_start() {
    let (s, _pool, token, [ana, rui, marta, _joana]) = formed(START).await;
    let (event_id, event_token) =
        event(&s, &ana, "Friday cup", START + 2 * HOUR, Signups::Public).await;
    s.join_event(&event_token, &rui, JoinAs::Solo, T0 + 5)
        .await
        .unwrap();
    let (_, match_token) = scheduled(&s, &marta, START + 4 * HOUR, Listing::Private).await;
    s.join_scheduled(&match_token, &ana, T0 + 6).await.unwrap();

    let c = cal(&s, &ana, T0, T0 + 10).await;
    assert_eq!(kinds(&c["items"]), ["game", "event", "match"]);
    let [game, ev, m] = [&c["items"][0], &c["items"][1], &c["items"][2]];
    assert_eq!(game["board_token"], token);
    assert_eq!(game["board_title"], "Thursday padel");
    assert_eq!(game["start"], START);
    assert_eq!(game["state"], "full");
    assert_eq!(game["players"].as_array().unwrap().len(), 4);
    assert_eq!(ev["event_id"], event_id.as_str());
    assert_eq!(ev["join_token"], event_token.as_str());
    assert_eq!(ev["name"], "Friday cup");
    assert_eq!(ev["event_kind"], "tournament");
    assert_eq!(ev["starts_at"], START + 2 * HOUR);
    assert_eq!(ev["place"], "Club");
    assert_eq!(ev["role"], "organiser");
    assert_eq!(ev["status"], "setup");
    assert_eq!(m["join_token"], match_token.as_str());
    assert_eq!(m["role"], "player");
    assert_eq!(m["state"], "open");
    assert_eq!(m["place"], "Court 2");
    assert_eq!(m["players"], json!(["Ana", null, null, null]));

    // Rui is signed up alone: a line item with no place, no event id, and only himself as team.
    let r = cal(&s, &rui, T0, T0 + 10).await;
    assert_eq!(kinds(&r["items"]), ["game", "event"]);
    let ev = &r["items"][1];
    assert_eq!(ev["role"], "line");
    assert!(ev["event_id"].is_null());
    assert!(ev["place_in_line"].is_null());
    assert_eq!(ev["team"], json!(["Rui"]));

    // Marta owns the match and is not in the event.
    let m = cal(&s, &marta, T0, T0 + 10).await;
    assert_eq!(kinds(&m["items"]), ["game", "match"]);
    assert_eq!(m["items"][1]["role"], "organiser");

    // Items before `from` are left out.
    let later = cal(&s, &ana, START + HOUR, T0 + 10).await;
    assert_eq!(kinds(&later["items"]), ["event", "match"]);
}

#[tokio::test]
async fn games_on_boards_i_left_are_gone() {
    let (s, _pool, token, [ana, rui, ..]) = formed(START).await;
    assert_eq!(kinds(&cal(&s, &rui, T0, T0 + 10).await["items"]), ["game"]);
    s.leave_board(&token, &rui, T0 + 20).await.unwrap();
    let after = cal(&s, &rui, T0, T0 + 30).await;
    assert!(after["items"].as_array().unwrap().is_empty());
    assert!(after["boards"].as_array().unwrap().is_empty());
    assert_eq!(kinds(&cal(&s, &ana, T0, T0 + 30).await["items"]), ["game"]);
}

#[tokio::test]
async fn over_boards_leave_the_calendar() {
    let (s, _pool, token, [ana, ..]) = formed(START).await;
    let c = cal(&s, &ana, T0, T0 + 10).await;
    let strips = c["boards"].as_array().unwrap();
    assert_eq!(strips.len(), 1);
    assert_eq!(strips[0]["token"], token);
    assert_eq!(strips[0]["title"], "Thursday padel");
    assert_eq!(strips[0]["starts_at"], START);
    assert_eq!(strips[0]["ends_at"], START + WEEK_SECS);
    assert_eq!(strips[0]["my_games"], 1);
    // The game is over but the board runs on: the strip stays, with no games left to play.
    let mid = cal(&s, &ana, T0, START + 2 * HOUR).await;
    assert_eq!(mid["boards"][0]["my_games"], 0);
    // Once the board is over the strip goes.
    let over = cal(&s, &ana, START, START + WEEK_SECS).await;
    assert!(over["boards"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn waiting_holds_swap_and_time_requests_and_my_partner_invite() {
    let (s, _pool, token, [a0, a1, a2, a3], [b0, ..]) = two_games().await;
    let view = s
        .board_view(&token, Some(&a0), T0 + 20)
        .await
        .unwrap()
        .unwrap();
    let (g1, g2) = (&view.games[0], &view.games[1]);
    s.ask_swap(&token, &g1.id, &a0, g1.version, &g2.id, &b0, T0 + 30)
        .await
        .unwrap();
    s.ask_time(
        &token,
        &g1.id,
        &a0,
        g1.version,
        START + 4 * HOUR,
        START + 5 * HOUR,
        false,
        T0 + 31,
    )
    .await
    .unwrap();

    // Rui... the asked player of the swap sees it with his own game first.
    let w = cal(&s, &b0, T0, T0 + 40).await["waiting"].clone();
    assert_eq!(kinds(&w), ["swap"]);
    assert_eq!(w[0]["board_token"], token);
    assert_eq!(w[0]["from_name"], "Aa0");
    assert_eq!(w[0]["my_game"], json!([g2.start, g2.end]));
    assert_eq!(w[0]["their_game"], json!([g1.start, g1.end]));
    assert!(w[0]["request_id"].is_string());

    // The others of the first game must answer the time change.
    let w = cal(&s, &a1, T0, T0 + 40).await["waiting"].clone();
    assert_eq!(kinds(&w), ["time"]);
    assert_eq!(w[0]["game"], json!([g1.start, g1.end]));
    assert_eq!(w[0]["new"], json!([START + 4 * HOUR, START + 5 * HOUR]));
    assert_eq!(w[0]["from_name"], "Aa0");
    assert_eq!(kinds(&cal(&s, &a2, T0, T0 + 40).await["waiting"]), ["time"]);
    assert_eq!(kinds(&cal(&s, &a3, T0, T0 + 40).await["waiting"]), ["time"]);
    // The asker has nothing to answer; once answered, a request leaves the list.
    assert!(
        cal(&s, &a0, T0, T0 + 40).await["waiting"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // A partner invitation is listed for the inviter, with the link to resend.
    let (_, event_token) = event(&s, &a0, "Friday cup", START + 2 * HOUR, Signups::Public).await;
    let rui = user(&s, "Rui").await;
    let entry = s
        .join_event(
            &event_token,
            &rui,
            JoinAs::Partner { partner_name: None },
            T0 + 50,
        )
        .await
        .unwrap();
    let partner_token = match entry {
        padel_score_server::competitions::signups::MyEntry::Pending { partner_token } => {
            partner_token
        }
        other => panic!("a link, not {other:?}"),
    };
    let w = cal(&s, &rui, T0, T0 + 60).await;
    assert_eq!(kinds(&w["waiting"]), ["partner"]);
    assert_eq!(w["waiting"][0]["join_token"], event_token.as_str());
    assert_eq!(w["waiting"][0]["event_name"], "Friday cup");
    assert_eq!(w["waiting"][0]["partner_token"], partner_token.as_str());
    // The pending entry is on the calendar too, with no place.
    assert_eq!(w["items"][0]["role"], "line");
}

#[tokio::test]
async fn waiting_leaves_out_stale_requests() {
    let (s, _pool, token, [a0, ..], [b0, ..]) = two_games().await;
    let view = s
        .board_view(&token, Some(&a0), T0 + 20)
        .await
        .unwrap()
        .unwrap();
    let (g1, g2) = (&view.games[0], &view.games[1]);
    s.ask_swap(&token, &g1.id, &a0, g1.version, &g2.id, &b0, T0 + 30)
        .await
        .unwrap();
    s.ask_time(
        &token,
        &g2.id,
        &b0,
        g2.version,
        START + 4 * HOUR,
        START + 5 * HOUR,
        false,
        T0 + 31,
    )
    .await
    .unwrap();
    assert_eq!(kinds(&cal(&s, &b0, T0, T0 + 40).await["waiting"]), ["swap"]);
    // Once the first game has started the swap is dead, though the tick has not set it expired.
    let late = START + 60;
    assert!(
        cal(&s, &b0, T0, late).await["waiting"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // The time request on the second game lives until that game starts.
    let b1 = &g2.players[1].user_id;
    assert_eq!(kinds(&cal(&s, b1, T0, T0 + 40).await["waiting"]), ["time"]);
    assert_eq!(kinds(&cal(&s, b1, T0, late).await["waiting"]), ["time"]);
    let started = START + 1900;
    assert!(
        cal(&s, b1, T0, started).await["waiting"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn open_to_join_lists_public_only_and_not_mine() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let me = user(&s, "Ana").await;
    let other = user(&s, "Rui").await;
    let (_, public_event) = event(&s, &owner, "Open cup", START + 2 * HOUR, Signups::Public).await;
    event(&s, &owner, "Link cup", START + 3 * HOUR, Signups::Private).await;
    let (_, mine) = event(&s, &other, "Signed cup", START + 5 * HOUR, Signups::Public).await;
    s.join_event(&mine, &me, JoinAs::Solo, T0 + 1)
        .await
        .unwrap();
    event(&s, &me, "My own cup", START + 6 * HOUR, Signups::Public).await;
    scheduled(&s, &owner, START + HOUR, Listing::Public).await;
    scheduled(&s, &owner, START + 7 * HOUR, Listing::Private).await;
    let (_, in_it) = scheduled(&s, &other, START + 8 * HOUR, Listing::Public).await;
    s.join_scheduled(&in_it, &me, T0 + 2).await.unwrap();
    scheduled(&s, &me, START + 9 * HOUR, Listing::Public).await;
    let (_, full) = scheduled(&s, &other, START + 10 * HOUR, Listing::Public).await;
    for n in ["P1", "P2", "P3", "P4"] {
        let id = user(&s, n).await;
        s.join_scheduled(&full, &id, T0 + 3).await.unwrap();
    }

    // One of three joined users to a public match also leaves a place.
    let c = cal(&s, &me, T0, T0 + 10).await;
    let open = c["open"].as_array().unwrap();
    assert_eq!(kinds(&c["open"]), ["match", "event"]);
    assert_eq!(open[0]["owner_name"], "Owner");
    assert_eq!(open[0]["starts_at"], START + HOUR);
    assert_eq!(open[0]["open_places"], 4);
    assert_eq!(open[1]["join_token"], public_event.as_str());
    assert_eq!(open[1]["name"], "Open cup");
    assert_eq!(open[1]["event_kind"], "tournament");
    assert_eq!(open[1]["teams"], 0);
    assert_eq!(open[1]["max_teams"], 8);
    // Not open once the deadline or the start has passed.
    let late = cal(&s, &me, T0, START + 2 * HOUR).await;
    assert!(
        late["open"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["kind"] == "match")
    );
}

#[tokio::test]
async fn private_event_is_not_open_to_join() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let me = user(&s, "Ana").await;
    event(&s, &owner, "Link cup", START + 3 * HOUR, Signups::Private).await;
    let c = cal(&s, &me, T0, T0 + 10).await;
    assert!(c["open"].as_array().unwrap().is_empty());
    assert!(c["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_event_shows_cancelled() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let me = user(&s, "Ana").await;
    let (id, token) = event(&s, &owner, "Friday cup", START + 3 * HOUR, Signups::Public).await;
    s.join_event(&token, &me, JoinAs::Solo, T0 + 1)
        .await
        .unwrap();
    s.cancel_competition(&owner, &id, T0 + 2).await.unwrap();
    for who in [&owner, &me] {
        let c = cal(&s, who, T0, T0 + 10).await;
        assert_eq!(kinds(&c["items"]), ["event"]);
        assert_eq!(c["items"][0]["status"], "cancelled");
    }
    // A cancelled event is not offered either.
    let other = user(&s, "Rui").await;
    assert!(
        cal(&s, &other, T0, T0 + 10).await["open"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn from_before_two_days_is_clamped() {
    let (state, pool) = state().await;
    let (id, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let app = app_from(&state, 1);
    let real = now();
    let mut ids = Vec::new();
    for offset in [-3 * DAY, -DAY, DAY] {
        let sid = state
            .store
            .create_scheduled(
                &id,
                NewScheduled {
                    starts_at: real + DAY,
                    place: None,
                    listing: Listing::Private,
                    format: one_set(),
                    places: Default::default(),
                },
                real,
            )
            .await
            .unwrap()
            .id;
        sqlx::query("UPDATE scheduled_matches SET starts_at = ? WHERE id = ?")
            .bind(real + offset)
            .bind(&sid)
            .execute(&pool)
            .await
            .unwrap();
        ids.push(sid);
    }
    let get = |path: &str| send(&app, as_session(&state, &raw, Method::GET, path, None));
    let res = get("/api/me/calendar?from=0").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    let starts: Vec<i64> = res.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["starts_at"].as_i64().unwrap())
        .collect();
    assert_eq!(starts, [real - DAY, real + DAY]);
    // A `from` after the clamp is used as given.
    let res = get(&format!("/api/me/calendar?from={}", real)).await;
    assert_eq!(res.body["items"].as_array().unwrap().len(), 1);
    // Missing or malformed is 400; signed out is 401.
    assert_eq!(
        get("/api/me/calendar").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get("/api/me/calendar?from=abc").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get("/api/me/calendar?from=").await.status,
        StatusCode::BAD_REQUEST
    );
    let anon = send(
        &app,
        support::req(Method::GET, "/api/me/calendar?from=0", None, &[]),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
}
