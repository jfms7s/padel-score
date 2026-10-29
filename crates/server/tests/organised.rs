//! `GET /api/me/organised`: what the caller runs. Store tests use an explicit clock from `T0`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::http::{Method, StatusCode, header};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::boards::{NewBoard, WEEK_SECS};
use padel_score_server::competitions::schedule::{ScheduleInput, Signups};
use padel_score_server::competitions::signups::JoinAs;
use padel_score_server::competitions::{Kind, NewCompetition};
use padel_score_server::db;
use padel_score_server::scheduled::{Listing, NewScheduled};
use padel_score_server::store::Store;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
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

async fn store() -> Store {
    Store::new(db::connect("sqlite::memory:").await.unwrap())
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

/// A board starting at `start`, made at `made_at`.
async fn board(s: &Store, creator: &str, start: i64, made_at: i64) -> String {
    s.create_board(
        creator,
        NewBoard {
            allow_guests: false,
            title: "Thursday padel".to_owned(),
            place: None,
            time_zone: "Europe/Lisbon".to_owned(),
            starts_at: start,
            ends_at: start + WEEK_SECS,
        },
        made_at,
    )
    .await
    .unwrap()
    .board
    .token
}

/// A tournament of at most four teams: (event id, join token).
async fn event(s: &Store, owner: &str, name: &str) -> (String, String) {
    let ev = s
        .create_competition(
            owner,
            NewCompetition {
                name: name.into(),
                kind: Kind::Tournament,
                format: one_set(),
                settings: None,
                schedule: Some(ScheduleInput {
                    starts_at: Some(Some(T0 + 3 * DAY)),
                    place: Some(Some("Club".to_owned())),
                    signups: Some(Signups::Public),
                    min_teams: Some(4),
                    max_teams: Some(Some(4)),
                    ..Default::default()
                }),
            },
            T0,
        )
        .await
        .unwrap();
    (ev.id, ev.join_token.unwrap())
}

async fn scheduled(s: &Store, owner: &str, start: i64) -> (String, String) {
    let v = s
        .create_scheduled(
            owner,
            NewScheduled {
                starts_at: start,
                place: Some("Court 2".to_owned()),
                listing: Listing::Private,
                format: one_set(),
                places: Default::default(),
            },
            T0,
        )
        .await
        .unwrap();
    (v.id, v.join_token)
}

async fn organised(s: &Store, who: &str, now: i64) -> Value {
    serde_json::to_value(s.organised(who, now).await.unwrap()).unwrap()
}

#[tokio::test]
async fn anonymous_is_401() {
    let (state, _pool) = state().await;
    let app = app_from(&state, 1);
    let res = send(
        &app,
        support::req(Method::GET, "/api/me/organised", None, &[]),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn lists_only_my_things_with_no_store() {
    let (state, _pool) = state().await;
    let (ana, raw) = fast_user(&state, "ana@x.test", "Ana").await;
    let (rui, _) = fast_user(&state, "rui@x.test", "Rui").await;
    let s = &state.store;
    event(s, &ana, "Ana cup").await;
    event(s, &rui, "Rui cup").await;
    let (_, ana_match) = scheduled(s, &ana, START).await;
    scheduled(s, &rui, START).await;
    let ana_board = board(s, &ana, START, T0).await;
    board(s, &rui, START, T0).await;

    let app = app_from(&state, 1);
    let res = send(
        &app,
        as_session(&state, &raw, Method::GET, "/api/me/organised", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(res.body["events"].as_array().unwrap().len(), 1);
    assert_eq!(res.body["matches"].as_array().unwrap().len(), 1);
    assert_eq!(res.body["boards"].as_array().unwrap().len(), 1);
    assert_eq!(res.body["events"][0]["name"], "Ana cup");
    assert_eq!(res.body["matches"][0]["join_token"], ana_match.as_str());
    assert_eq!(res.body["boards"][0]["token"], ana_board.as_str());
}

#[tokio::test]
async fn event_counts_solos_and_line() {
    let s = store().await;
    let ana = user(&s, "Ana").await;
    let (id, token) = event(&s, &ana, "Cup").await;
    for i in 0..5 {
        let p = user(&s, &format!("P{i}")).await;
        s.join_event(
            &token,
            &p,
            JoinAs::Partner {
                partner_name: Some(format!("Mate{i}")),
            },
            T0 + i,
        )
        .await
        .unwrap();
    }
    let solo = user(&s, "Solo").await;
    s.join_event(&token, &solo, JoinAs::Solo, T0 + 10)
        .await
        .unwrap();

    let o = organised(&s, &ana, T0 + 20).await;
    let ev = &o["events"][0];
    assert_eq!(ev["id"], id.as_str());
    assert_eq!(ev["status"], "setup");
    assert!(ev["join_token"].is_string());
    assert_eq!(ev["signed_teams"], 4);
    assert_eq!(ev["line_count"], 1);
    assert_eq!(ev["solo_count"], 1);
}

#[tokio::test]
async fn match_states_and_places() {
    let s = store().await;
    let ana = user(&s, "Ana").await;
    let (_, open) = scheduled(&s, &ana, START).await;
    s.join_scheduled(&open, &ana, T0 + 1).await.unwrap();
    let (_, full) = scheduled(&s, &ana, START + HOUR).await;
    for i in 0..5 {
        let p = user(&s, &format!("P{i}")).await;
        s.join_scheduled(&full, &p, T0 + 2 + i).await.unwrap();
    }
    let (cancelled_id, _) = scheduled(&s, &ana, START + 2 * HOUR).await;
    s.cancel_scheduled(&ana, &cancelled_id, T0 + 3)
        .await
        .unwrap();

    let o = organised(&s, &ana, T0 + 20).await;
    let by = |token: &str| {
        o["matches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["join_token"] == token)
            .unwrap()
            .clone()
    };
    let m = by(&open);
    assert_eq!(m["state"], "open");
    assert_eq!(m["places"], json!(["Ana", null, null, null]));
    assert_eq!(m["line_count"], 0);
    assert_eq!(m["place"], "Court 2");
    let m = by(&full);
    assert_eq!(m["state"], "full");
    assert_eq!(m["line_count"], 1);
    // Newest start first; the cancelled one starts last.
    assert_eq!(o["matches"][0]["state"], "cancelled");
}

#[tokio::test]
async fn boards_include_over_and_left() {
    let s = store().await;
    let ana = user(&s, "Ana").await;
    let rui = user(&s, "Rui").await;
    let marta = user(&s, "Marta").await;
    let joana = user(&s, "Joana").await;
    let live = board(&s, &ana, START, T0).await;
    for who in [&rui, &marta, &joana] {
        s.join_board(&live, who, T0 + 1).await.unwrap();
    }
    for (i, who) in [&ana, &rui, &marta, &joana].into_iter().enumerate() {
        s.add_free(&live, who, START, START + HOUR, T0 + 2 + i as i64)
            .await
            .unwrap();
    }
    let old = board(&s, &ana, T0 - 2 * WEEK_SECS, T0 - 2 * WEEK_SECS).await;
    let left = board(&s, &ana, START + DAY, T0).await;
    s.leave_board(&left, &ana, T0 + 5).await.unwrap();

    let o = organised(&s, &ana, T0 + 20).await;
    let boards = o["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 3);
    let by = |token: &str| boards.iter().find(|b| b["token"] == token).unwrap();
    let b = by(&live);
    assert_eq!(b["over"], false);
    assert_eq!(b["member_count"], 4);
    assert_eq!(b["game_count"], 1);
    assert_eq!(b["title"], "Thursday padel");
    assert_eq!(by(&old)["over"], true);
    assert_eq!(by(&left)["member_count"], 0);
    // Newest start first.
    assert_eq!(boards[0]["token"], left.as_str());
}
