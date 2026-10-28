#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! A guest acting on a board through the API with the `X-Board-Guest` header: which routes take
//! it, that it stays on its own board, and that a session or a device token keeps its meaning.

mod support;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use padel_score_server::AppState;
use padel_score_server::api::now;
use padel_score_server::boards::{NewBoard, WEEK_SECS};
use serde_json::{Value, json};
use support::{as_device, as_session, cookie, device_token, fast_user, req, send, state};

const HOUR: i64 = 3600;
const HALF: i64 = 1800;
const KEY: &str = "x-board-guest";

/// The start of a week that began an hour ago, on the half-hour grid.
fn week_start() -> i64 {
    now() / HALF * HALF - HOUR
}

/// A board with guests allowed, made through the store for `creator`: its token.
async fn guest_board(state: &AppState, creator: &str) -> String {
    let start = week_start();
    state
        .store
        .create_board(
            creator,
            NewBoard {
                allow_guests: true,
                title: "Guests".to_owned(),
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

/// A guest joined through the store: (guest id, raw key).
async fn guest(state: &AppState, token: &str, name: &str) -> (String, String) {
    let joined = state.store.join_as_guest(token, name, now()).await.unwrap();
    let id = joined.view.me.unwrap().user_id;
    (id, joined.guest_key)
}

fn as_guest(key: &str, method: Method, path: &str, body: Option<Value>) -> Request<Body> {
    req(method, path, body, &[(KEY, key)])
}

fn range(start: i64) -> Value {
    json!({ "start": start, "end": start + HOUR })
}

/// A state, its router, a board with guests on, and its creator's id and session.
async fn setup() -> (AppState, Router, String, String, String) {
    let (state, _) = state().await;
    let app = support::app_from(&state, 1);
    let (a, a_raw) = fast_user(&state, "a@x.test", "Ana").await;
    let token = guest_board(&state, &a).await;
    (state, app, token, a, a_raw)
}

/// The game starting at `start` in a view body.
fn game_at(view: &Value, start: i64) -> Value {
    view["games"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["start"] == start)
        .unwrap_or_else(|| panic!("no game at {start}: {view}"))
        .clone()
}

fn seated(game: &Value, id: &str) -> bool {
    game["players"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["user_id"] == id)
}

/// `n` new users each add free time over `start..start + HOUR`: the last view.
async fn users_add(state: &AppState, app: &Router, token: &str, start: i64, n: usize) -> Value {
    let path = format!("/api/boards/{token}/ranges");
    let mut last = Value::Null;
    for i in 0..n {
        let (_, raw) = fast_user(state, &format!("u{start}-{i}@x.test"), &format!("User{i}")).await;
        let res = send(
            app,
            as_session(state, &raw, Method::POST, &path, Some(range(start))),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        last = res.body;
    }
    last
}

#[tokio::test]
async fn guest_adds_and_removes_free_time() {
    let (state, app, token, ..) = setup().await;
    let (gid, key) = guest(&state, &token, "Gus").await;
    let path = format!("/api/boards/{token}/ranges");
    let start = week_start() + 7200;
    let add = send(
        &app,
        as_guest(&key, Method::POST, &path, Some(range(start))),
    )
    .await;
    assert_eq!(add.status, StatusCode::OK, "{}", add.body);
    assert_eq!(add.body["me"]["guest"], true);
    assert_eq!(add.body["me"]["user_id"], gid.as_str());
    let pieces = add.body["pieces"].as_array().unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0]["user_id"], gid.as_str());

    let remove = send(
        &app,
        as_guest(
            &key,
            Method::POST,
            &format!("{path}/remove"),
            Some(range(start)),
        ),
    )
    .await;
    assert_eq!(remove.status, StatusCode::OK, "{}", remove.body);
    assert!(remove.body["pieces"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn guest_key_from_another_board_is_401() {
    let (state, app, token_a, ..) = setup().await;
    let (b, _) = fast_user(&state, "b@x.test", "Bia").await;
    let token_b = guest_board(&state, &b).await;
    let (_, key_a) = guest(&state, &token_a, "Gus").await;

    let add = send(
        &app,
        as_guest(
            &key_a,
            Method::POST,
            &format!("/api/boards/{token_b}/ranges"),
            Some(range(week_start() + 7200)),
        ),
    )
    .await;
    assert_eq!(add.status, StatusCode::UNAUTHORIZED);

    let read = send(
        &app,
        as_guest(&key_a, Method::GET, &format!("/api/boards/{token_b}"), None),
    )
    .await;
    assert_eq!(read.status, StatusCode::OK);
    assert!(read.body["me"].is_null(), "{}", read.body);
    assert!(read.body["pieces"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn guest_cannot_start_patch_delete_or_join() {
    let (state, app, token, ..) = setup().await;
    let (_, key) = guest(&state, &token, "Gus").await;
    let base = format!("/api/boards/{token}");
    for (method, path, body) in [
        (Method::POST, format!("{base}/games/nope/start"), None),
        (Method::PATCH, base.clone(), Some(json!({ "title": "x" }))),
        (Method::DELETE, base.clone(), None),
        (Method::POST, format!("{base}/join"), None),
    ] {
        let res = send(&app, as_guest(&key, method.clone(), &path, body)).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[tokio::test]
async fn session_wins_over_a_guest_header() {
    let (state, app, token, _, a_raw) = setup().await;
    let (_, key) = guest(&state, &token, "Gus").await;
    let headers = [("cookie", cookie(&state, &a_raw)), (KEY, key)];
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let write = send(
        &app,
        req(
            Method::POST,
            &format!("/api/boards/{token}/ranges"),
            Some(range(week_start() + 7200)),
            &headers,
        ),
    )
    .await;
    assert_eq!(write.status, StatusCode::OK, "{}", write.body);
    assert_eq!(write.body["me"]["guest"], false);
    let read = send(
        &app,
        req(Method::GET, &format!("/api/boards/{token}"), None, &headers),
    )
    .await;
    assert_eq!(read.body["me"]["guest"], false);
}

#[tokio::test]
async fn dead_session_cookie_falls_back_to_the_guest_key() {
    let (state, app, token, ..) = setup().await;
    let (gid, key) = guest(&state, &token, "Gus").await;
    let dead = cookie(&state, "made-up-session");
    let res = send(
        &app,
        req(
            Method::POST,
            &format!("/api/boards/{token}/ranges"),
            Some(range(week_start() + 7200)),
            &[("cookie", &dead), (KEY, &key)],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["me"]["user_id"], gid.as_str());
    assert_eq!(res.body["me"]["guest"], true);
}

#[tokio::test]
async fn device_token_still_403_on_writes() {
    let (state, app, token, a, _) = setup().await;
    let device = device_token(&state.store, &a).await;
    let path = format!("/api/boards/{token}/ranges");
    let body = Some(range(week_start() + 7200));
    let res = send(&app, as_device(&device, Method::POST, &path, body.clone())).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // A guest key beside the device token does not turn it into a guest.
    let (_, key) = guest(&state, &token, "Gus").await;
    let bearer = format!("Bearer {device}");
    let res = send(
        &app,
        req(
            Method::POST,
            &path,
            body,
            &[("authorization", &bearer), (KEY, &key)],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn read_with_a_dead_key_is_signed_out() {
    let (state, app, token, creator, _) = setup().await;
    let (gid, key) = guest(&state, &token, "Gus").await;
    let path = format!("/api/boards/{token}");
    let live = send(&app, as_guest(&key, Method::GET, &path, None)).await;
    assert_eq!(live.body["me"]["user_id"], gid.as_str());

    state
        .store
        .remove_guest(&token, &creator, &gid, now())
        .await
        .unwrap();
    let dead = send(&app, as_guest(&key, Method::GET, &path, None)).await;
    assert_eq!(dead.status, StatusCode::OK);
    assert!(dead.body["me"].is_null(), "{}", dead.body);
    let write = send(
        &app,
        as_guest(
            &key,
            Method::POST,
            &format!("{path}/ranges"),
            Some(range(week_start() + 7200)),
        ),
    )
    .await;
    assert_eq!(write.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn guest_takes_and_gives_a_place() {
    let (state, app, token, ..) = setup().await;
    let floor = now() / HALF * HALF;
    let ranges = format!("/api/boards/{token}/ranges");

    // A game over an hour away with a fifth person in the line: the guest, seated, gives.
    let far = floor + 3 * HOUR;
    let (gid, key) = guest(&state, &token, "Gus").await;
    let res = send(
        &app,
        as_guest(&key, Method::POST, &ranges, Some(range(far))),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let view = users_add(&state, &app, &token, far, 4).await;
    let game = game_at(&view, far);
    assert!(seated(&game, &gid), "{game}");
    let give = send(
        &app,
        as_guest(
            &key,
            Method::POST,
            &format!(
                "/api/boards/{token}/games/{}/give",
                game["id"].as_str().unwrap()
            ),
            Some(json!({ "version": game["version"] })),
        ),
    )
    .await;
    assert_eq!(give.status, StatusCode::OK, "{}", give.body);
    assert!(!seated(&game_at(&give.body, far), &gid), "{}", give.body);

    // A game with nobody in line loses a player; a second guest takes the open place.
    let second = floor + 5 * HOUR;
    let (taker, taker_key) = guest(&state, &token, "Gabi").await;
    let view = users_add(&state, &app, &token, second, 4).await;
    let game = game_at(&view, second);
    let leaver = game["players"][0]["user_id"].as_str().unwrap().to_owned();
    let leaver_raw = state.store.create_session(&leaver, now()).await.unwrap();
    let leave = send(
        &app,
        as_session(
            &state,
            &leaver_raw,
            Method::POST,
            &format!(
                "/api/boards/{token}/games/{}/leave",
                game["id"].as_str().unwrap()
            ),
            Some(json!({ "version": game["version"] })),
        ),
    )
    .await;
    assert_eq!(leave.status, StatusCode::OK, "{}", leave.body);
    let game = game_at(&leave.body, second);
    assert_eq!(game["state"], "short", "{game}");
    let take = send(
        &app,
        as_guest(
            &taker_key,
            Method::POST,
            &format!(
                "/api/boards/{token}/games/{}/take",
                game["id"].as_str().unwrap()
            ),
            Some(json!({ "version": game["version"] })),
        ),
    )
    .await;
    assert_eq!(take.status, StatusCode::OK, "{}", take.body);
    assert_eq!(take.body["me"]["guest"], true);
    let taken = game_at(&take.body, second);
    assert_eq!(taken["state"], "full", "{taken}");
    assert!(seated(&taken, &taker), "{taken}");
}

fn post_guest(token: &str, name: &str) -> Request<Body> {
    req(
        Method::POST,
        &format!("/api/boards/{token}/guests"),
        Some(json!({ "name": name })),
        &[],
    )
}

#[tokio::test]
async fn join_as_guest_route() {
    let (state, app, token, ..) = setup().await;
    let res = send(&app, post_guest(&token, "Gus")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert!(!res.body["guest_key"].as_str().unwrap().is_empty());
    assert_eq!(res.body["view"]["me"]["guest"], true);

    let (b, _) = fast_user(&state, "b@x.test", "Bia").await;
    let start = week_start();
    let closed = state
        .store
        .create_board(
            &b,
            NewBoard {
                allow_guests: false,
                title: "Closed".to_owned(),
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
        .token;
    let off = send(&app, post_guest(&closed, "Gus")).await;
    assert_eq!(off.status, StatusCode::CONFLICT);
    assert_eq!(off.body["error"], "guests_off");

    let empty = send(&app, post_guest(&token, "   ")).await;
    assert_eq!(
        empty.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        empty.body
    );
    assert_eq!(empty.body["field"], "name");
}

#[tokio::test]
async fn pick_route_gives_a_working_key() {
    let (state, app, token, ..) = setup().await;
    let (id, _) = guest(&state, &token, "Gus").await;
    let res = send(
        &app,
        req(
            Method::POST,
            &format!("/api/boards/{token}/guests/{id}/pick"),
            None,
            &[],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let key = res.body["guest_key"].as_str().unwrap().to_owned();
    let add = send(
        &app,
        as_guest(
            &key,
            Method::POST,
            &format!("/api/boards/{token}/ranges"),
            Some(range(week_start() + 7200)),
        ),
    )
    .await;
    assert_eq!(add.status, StatusCode::OK, "{}", add.body);
}

#[tokio::test]
async fn remove_route_is_creator_only() {
    let (state, app, token, _, a_raw) = setup().await;
    let (id, _) = guest(&state, &token, "Gus").await;
    let (_, b_raw) = fast_user(&state, "b@x.test", "Bia").await;
    let path = format!("/api/boards/{token}/guests/{id}");
    let join = send(
        &app,
        as_session(
            &state,
            &b_raw,
            Method::POST,
            &format!("/api/boards/{token}/join"),
            None,
        ),
    )
    .await;
    assert_eq!(join.status, StatusCode::OK, "{}", join.body);

    let denied = send(
        &app,
        as_session(&state, &b_raw, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(denied.status, StatusCode::CONFLICT);
    assert_eq!(denied.body["error"], "creator_only");

    let ok = send(
        &app,
        as_session(&state, &a_raw, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
}

#[tokio::test]
async fn claim_route_moves_the_guest() {
    let (state, app, token, ..) = setup().await;
    let (id, _) = guest(&state, &token, "Gus").await;
    let (b, b_raw) = fast_user(&state, "b@x.test", "Bia").await;
    let res = send(
        &app,
        as_session(
            &state,
            &b_raw,
            Method::POST,
            &format!("/api/boards/{token}/guests/{id}/claim"),
            None,
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let members = res.body["members"].as_array().unwrap();
    assert!(members.iter().all(|m| m["user_id"] != id), "{members:?}");
    assert!(members.iter().any(|m| m["user_id"] == b), "{members:?}");
}

#[tokio::test]
async fn guest_join_is_rate_limited() {
    let (state, app, token, ..) = setup().await;
    for i in 0..20 {
        let res = send(&app, post_guest(&token, &format!("Gus{i}"))).await;
        assert_eq!(res.status, StatusCode::CREATED, "{i}: {}", res.body);
    }
    let limited = send(&app, post_guest(&token, "Gus20")).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    let other = support::app_from(&state, 2);
    let res = send(&other, post_guest(&token, "Gus21")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[tokio::test]
async fn guest_routes_send_secret_headers() {
    let (_state, app, token, ..) = setup().await;
    let res = send(&app, post_guest(&token, "Gus")).await;
    assert_eq!(res.status, StatusCode::CREATED);
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
    assert_eq!(
        res.header(axum::http::header::REFERRER_POLICY),
        Some("no-referrer")
    );
}

#[tokio::test]
async fn cross_site_cookie_with_guest_key_is_403() {
    let (state, app, token, _, a_raw) = setup().await;
    let (_, key) = guest(&state, &token, "Gus").await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/boards/{token}/ranges"))
        .header("sec-fetch-site", "cross-site")
        .header("cookie", cookie(&state, &a_raw))
        .header(KEY, key)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(range(week_start() + 7200).to_string()))
        .unwrap();
    let res = send(&app, request).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    let view = send(
        &app,
        as_session(
            &state,
            &a_raw,
            Method::GET,
            &format!("/api/boards/{token}"),
            None,
        ),
    )
    .await;
    assert!(view.body["pieces"].as_array().unwrap().is_empty());
}
