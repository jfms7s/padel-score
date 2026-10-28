#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The board routes: auth per route, secret headers, the range round trip, a game forming
//! through the API and the caller's own boards.

mod support;

use axum::http::{Method, StatusCode, header};
use padel_score_server::api::now;
use padel_score_server::boards::{NewBoard, WEEK_SECS};
use serde_json::{Value, json};
use support::{as_device, as_session, device_token, fast_user, req, send, state};

const HOUR: i64 = 3600;

/// A week that started an hour ago, on the half-hour grid.
fn week_start() -> i64 {
    now() / 1800 * 1800 - HOUR
}

fn new_board_json() -> Value {
    let start = week_start();
    json!({
        "title": "Thursday padel", "place": "Club", "time_zone": "Europe/Lisbon",
        "starts_at": start, "ends_at": start + WEEK_SECS,
    })
}

/// A board made through the store, for `creator`: its token.
async fn seed_board(state: &padel_score_server::AppState, creator: &str) -> String {
    let start = week_start();
    state
        .store
        .create_board(
            creator,
            NewBoard {
                allow_guests: false,
                title: "Seeded".to_owned(),
                place: None,
                time_zone: "Europe/Lisbon".to_owned(),
                starts_at: start,
                ends_at: start + WEEK_SECS,
            },
            now() - 7200,
        )
        .await
        .unwrap()
        .board
        .token
}

#[tokio::test]
async fn create_needs_a_session() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, _) = fast_user(&state, "a@x.test", "Ana").await;
    let anon = send(
        &app,
        req(Method::POST, "/api/boards", Some(new_board_json()), &[]),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let token = device_token(&state.store, &id).await;
    let device = send(
        &app,
        as_device(&token, Method::POST, "/api/boards", Some(new_board_json())),
    )
    .await;
    assert_eq!(device.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_gives_201_and_the_view() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (_, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/boards",
            Some(new_board_json()),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["board"]["title"], "Thursday padel");
    assert_eq!(res.body["me"]["creator"], true);
}

#[tokio::test]
async fn signed_out_reads_the_board_read_only() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, _) = fast_user(&state, "a@x.test", "Ana").await;
    let token = seed_board(&state, &id).await;
    let path = format!("/api/boards/{token}");
    let res = send(&app, req(Method::GET, &path, None, &[])).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.body["me"].is_null());
    assert_eq!(res.body["board"]["token"], token.as_str());
    let start = week_start();
    let add = send(
        &app,
        req(
            Method::POST,
            &format!("{path}/ranges"),
            Some(json!({ "start": start + 7200, "end": start + 10800 })),
            &[],
        ),
    )
    .await;
    assert_eq!(add.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_device_token_reads_but_cannot_write() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, _) = fast_user(&state, "a@x.test", "Ana").await;
    let token = seed_board(&state, &id).await;
    let device = device_token(&state.store, &id).await;
    let path = format!("/api/boards/{token}");
    let read = send(&app, as_device(&device, Method::GET, &path, None)).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.body["me"]["user_id"], id.as_str());
    let del = send(&app, as_device(&device, Method::DELETE, &path, None)).await;
    assert_eq!(del.status, StatusCode::FORBIDDEN);
    let mine = send(
        &app,
        as_device(&device, Method::GET, "/api/me/boards", None),
    )
    .await;
    assert_eq!(mine.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn board_responses_carry_no_store_and_no_referrer() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let token = seed_board(&state, &id).await;
    let created = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/boards",
            Some(new_board_json()),
        ),
    )
    .await;
    let read = send(
        &app,
        req(Method::GET, &format!("/api/boards/{token}"), None, &[]),
    )
    .await;
    let mine = send(
        &app,
        as_session(&state, &raw, Method::GET, "/api/me/boards", None),
    )
    .await;
    let missing = send(&app, req(Method::GET, "/api/boards/nope", None, &[])).await;
    for res in [&created, &read, &mine, &missing] {
        assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
        assert_eq!(res.header(header::REFERRER_POLICY), Some("no-referrer"));
    }
}

#[tokio::test]
async fn range_round_trip() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let token = seed_board(&state, &id).await;
    let start = week_start() + 7200;
    let body = json!({ "start": start, "end": start + HOUR });
    let path = format!("/api/boards/{token}/ranges");
    let added = send(
        &app,
        as_session(&state, &raw, Method::POST, &path, Some(body.clone())),
    )
    .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.body);
    let pieces = added.body["pieces"].as_array().unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0]["start"], start);
    assert_eq!(pieces[0]["end"], start + HOUR);
    let removed = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            &format!("{path}/remove"),
            Some(body),
        ),
    )
    .await;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.body);
    assert!(removed.body["pieces"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn bad_range_is_422_and_a_non_creator_patch_is_409() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let (_, other) = fast_user(&state, "b@x.test", "Bia").await;
    let token = seed_board(&state, &id).await;
    let bad = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            &format!("/api/boards/{token}/ranges"),
            Some(json!({ "start": 1, "end": 2 })),
        ),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    let patch = send(
        &app,
        as_session(
            &state,
            &other,
            Method::PATCH,
            &format!("/api/boards/{token}"),
            Some(json!({ "title": "Mine now" })),
        ),
    )
    .await;
    assert_eq!(patch.status, StatusCode::CONFLICT);
    assert_eq!(patch.body["error"], "creator_only");
}

#[tokio::test]
async fn patch_join_leave_and_delete() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (id, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let (_, other) = fast_user(&state, "b@x.test", "Bia").await;
    let token = seed_board(&state, &id).await;
    let path = format!("/api/boards/{token}");
    let patched = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::PATCH,
            &path,
            Some(json!({ "title": "Renamed", "place": null })),
        ),
    )
    .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.body);
    assert_eq!(patched.body["board"]["title"], "Renamed");
    let joined = send(
        &app,
        as_session(&state, &other, Method::POST, &format!("{path}/join"), None),
    )
    .await;
    assert_eq!(joined.status, StatusCode::OK);
    assert_eq!(joined.body["members"].as_array().unwrap().len(), 2);
    let left = send(
        &app,
        as_session(&state, &other, Method::POST, &format!("{path}/leave"), None),
    )
    .await;
    assert_eq!(left.status, StatusCode::NO_CONTENT);
    let deleted = send(&app, as_session(&state, &raw, Method::DELETE, &path, None)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = send(&app, req(Method::GET, &path, None, &[])).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn four_users_form_a_game_through_the_api() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (a, a_raw) = fast_user(&state, "a@x.test", "Ana").await;
    let token = seed_board(&state, &a).await;
    let start = week_start() + 7200;
    let body = json!({ "start": start, "end": start + 90 * 60 });
    let path = format!("/api/boards/{token}/ranges");
    let mut last = Value::Null;
    let mut sessions = vec![a_raw];
    for (i, name) in ["Bia", "Cai", "Dani"].iter().enumerate() {
        let (_, raw) = fast_user(&state, &format!("u{i}@x.test"), name).await;
        sessions.push(raw);
    }
    for raw in &sessions {
        let res = send(
            &app,
            as_session(&state, raw, Method::POST, &path, Some(body.clone())),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        last = res.body;
    }
    let games = last["games"].as_array().unwrap();
    assert_eq!(games[0]["state"], "full", "{last}");
    assert_eq!(games[0]["players"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn unknown_token_is_404() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (_, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let get = send(&app, req(Method::GET, "/api/boards/nope", None, &[])).await;
    assert_eq!(get.status, StatusCode::NOT_FOUND);
    for (method, suffix, body) in [
        (Method::POST, "/join", None),
        (Method::POST, "/leave", None),
        (Method::PATCH, "", Some(json!({ "title": "x" }))),
        (Method::DELETE, "", None),
        (
            Method::POST,
            "/ranges",
            Some(json!({ "start": 0, "end": 1800 })),
        ),
        (
            Method::POST,
            "/ranges/remove",
            Some(json!({ "start": 0, "end": 1800 })),
        ),
    ] {
        let res = send(
            &app,
            as_session(
                &state,
                &raw,
                method.clone(),
                &format!("/api/boards/nope{suffix}"),
                body,
            ),
        )
        .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {suffix}");
    }
}

#[tokio::test]
async fn my_boards_lists_joined_boards_only() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (a, a_raw) = fast_user(&state, "a@x.test", "Ana").await;
    let (b, b_raw) = fast_user(&state, "b@x.test", "Bia").await;
    let mine = seed_board(&state, &a).await;
    seed_board(&state, &b).await;
    let res = send(
        &app,
        as_session(&state, &a_raw, Method::GET, "/api/me/boards", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let items = res.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["token"], mine.as_str());
    let anon = send(&app, req(Method::GET, "/api/me/boards", None, &[])).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let _ = b_raw;
}

#[tokio::test]
async fn game_actions_need_a_session_and_a_fresh_version() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let mut users = Vec::new();
    for (i, name) in ["Ana", "Rui", "Marta", "Joana"].into_iter().enumerate() {
        users.push(fast_user(&state, &format!("u{i}@x.test"), name).await);
    }
    let token = seed_board(&state, &users[0].0).await;
    // A game a few hours out, formed by a seed earlier than now so it may form.
    let start = week_start() + 8 * HOUR;
    for (id, _) in &users {
        state
            .store
            .add_free(&token, id, start, start + HOUR, now() - 7200)
            .await
            .unwrap();
    }
    let view = state
        .store
        .board_view(&token, Some(&users[0].0), now())
        .await
        .unwrap()
        .unwrap();
    let game = &view.games[0];
    let path = |action: &str| format!("/api/boards/{token}/games/{}/{action}", game.id);

    let anon = send(
        &app,
        req(
            Method::POST,
            &path("leave"),
            Some(json!({ "version": game.version })),
            &[],
        ),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);

    let stale = send(
        &app,
        as_session(
            &state,
            &users[1].1,
            Method::POST,
            &path("give"),
            Some(json!({ "version": game.version + 1 })),
        ),
    )
    .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.body["error"], "game_changed");

    let nobody = send(
        &app,
        as_session(
            &state,
            &users[1].1,
            Method::POST,
            &path("give"),
            Some(json!({ "version": game.version })),
        ),
    )
    .await;
    assert_eq!(nobody.status, StatusCode::CONFLICT);
    assert_eq!(nobody.body["error"], "nobody_can_take");

    let left = send(
        &app,
        as_session(
            &state,
            &users[1].1,
            Method::POST,
            &path("leave"),
            Some(json!({ "version": game.version })),
        ),
    )
    .await;
    assert_eq!(left.status, StatusCode::OK, "{}", left.body);
    assert_eq!(left.body["games"][0]["state"], "short");
    assert_eq!(
        left.body["games"][0]["players"].as_array().unwrap().len(),
        3
    );
    assert_eq!(left.header(header::CACHE_CONTROL), Some("no-store"));
    let version = left.body["games"][0]["version"].as_i64().unwrap();

    let taken = send(
        &app,
        as_session(
            &state,
            &users[1].1,
            Method::POST,
            &path("take"),
            Some(json!({ "version": version })),
        ),
    )
    .await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.body);
    assert_eq!(taken.body["games"][0]["state"], "full");

    let missing = send(
        &app,
        as_session(
            &state,
            &users[1].1,
            Method::POST,
            &format!("/api/boards/{token}/games/nope/take"),
            Some(json!({ "version": 0 })),
        ),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn time_change_requests_through_the_routes() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let mut users = Vec::new();
    for (i, name) in ["Ana", "Rui", "Marta", "Joana"].into_iter().enumerate() {
        users.push(fast_user(&state, &format!("u{i}@x.test"), name).await);
    }
    let token = seed_board(&state, &users[0].0).await;
    let start = week_start() + 8 * HOUR;
    for (id, _) in &users {
        state
            .store
            .add_free(&token, id, start, start + HOUR, now() - 7200)
            .await
            .unwrap();
    }
    let view = state
        .store
        .board_view(&token, Some(&users[0].0), now())
        .await
        .unwrap()
        .unwrap();
    let game = &view.games[0];
    let path = format!("/api/boards/{token}/games/{}/time", game.id);
    let body = json!({
        "version": game.version, "start": start + 2 * HOUR, "end": start + 3 * HOUR,
        "agreed": false,
    });
    let anon = send(&app, req(Method::POST, &path, Some(body.clone()), &[])).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let bad = send(
        &app,
        as_session(
            &state,
            &users[0].1,
            Method::POST,
            &path,
            Some(json!({ "version": game.version, "start": start + 2 * HOUR,
                         "end": start + 2 * HOUR + 1800 })),
        ),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad.body["field"], "range");
    let asked = send(
        &app,
        as_session(&state, &users[0].1, Method::POST, &path, Some(body)),
    )
    .await;
    assert_eq!(asked.status, StatusCode::OK, "{}", asked.body);
    assert_eq!(asked.body["requests"][0]["kind"], "time");
    assert_eq!(asked.body["requests"][0]["needs_my_answer"], false);
    let id = asked.body["requests"][0]["id"].as_str().unwrap().to_owned();
    let answer = |who: usize, action: &str| {
        as_session(
            &state,
            &users[who].1,
            Method::POST,
            &format!("/api/board-requests/{id}/{action}"),
            None,
        )
    };
    // The asker is not an addressee, and signed out has no session.
    let own = send(&app, answer(0, "accept")).await;
    assert_eq!(own.status, StatusCode::NOT_FOUND);
    let anon = send(
        &app,
        req(
            Method::POST,
            &format!("/api/board-requests/{id}/accept"),
            None,
            &[],
        ),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    for who in [1, 2] {
        let res = send(&app, answer(who, "accept")).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body["games"][0]["moved"], false);
        assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    }
    let done = send(&app, answer(3, "accept")).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(done.body["games"][0]["moved"], true);
    assert_eq!(done.body["games"][0]["start"], start + 2 * HOUR);
    assert!(done.body["requests"].as_array().unwrap().is_empty());
    // Answered: nothing is pending any more.
    let again = send(&app, answer(3, "decline")).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    // A stale version is the same 409 as for the other game actions.
    let stale = send(
        &app,
        as_session(
            &state,
            &users[0].1,
            Method::POST,
            &path,
            Some(json!({ "version": game.version, "start": start + 4 * HOUR,
                         "end": start + 5 * HOUR, "agreed": true })),
        ),
    )
    .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.body["error"], "game_changed");
}

#[tokio::test]
async fn create_json_without_allow_guests_is_off() {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (_, raw) = fast_user(&state, "a@x.test", "Ana").await;
    let post = |body: Value| as_session(&state, &raw, Method::POST, "/api/boards", Some(body));
    let off = send(&app, post(new_board_json())).await;
    assert_eq!(off.status, StatusCode::CREATED, "{}", off.body);
    assert_eq!(off.body["board"]["allow_guests"], false);
    let mut body = new_board_json();
    body["allow_guests"] = json!(true);
    let on = send(&app, post(body)).await;
    assert_eq!(on.status, StatusCode::CREATED, "{}", on.body);
    assert_eq!(on.body["board"]["allow_guests"], true);
}
