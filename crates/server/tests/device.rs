#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The device-code sign-in flow (`/api/device/*`) and device management (`/api/me/devices`),
//! through the real router.

mod support;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use padel_score_server::AppState;
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use support::*;

/// A request as the watch sends it: JSON, no browser headers, no cookie.
fn watch_post(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// `POST /api/device/start` with `body`, asserting 200.
async fn start_with(app: &Router, body: Value) -> Value {
    let res = send(app, watch_post("/api/device/start", body)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

async fn start(app: &Router) -> Value {
    start_with(app, json!({ "device_name": "Galaxy Watch6" })).await
}

async fn poll(app: &Router, device_code: &str) -> Res {
    send(
        app,
        watch_post("/api/device/token", json!({ "device_code": device_code })),
    )
    .await
}

fn code_of(started: &Value, key: &str) -> String {
    started[key].as_str().unwrap().to_owned()
}

async fn lookup(state: &AppState, app: &Router, raw: &str, user_code: &str) -> Res {
    let path = format!("/api/device/{user_code}");
    send(app, as_session(state, raw, Method::GET, &path, None)).await
}

async fn decide(state: &AppState, app: &Router, raw: &str, user_code: &str, verb: &str) -> Res {
    let path = format!("/api/device/{user_code}/{verb}");
    send(app, as_session(state, raw, Method::POST, &path, None)).await
}

async fn approve(state: &AppState, app: &Router, raw: &str, user_code: &str) -> Res {
    decide(state, app, raw, user_code, "approve").await
}

fn assert_poll_error(res: &Res, code: &str) {
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.body);
    assert_eq!(res.body, json!({ "error": code }));
}

/// Run the whole flow over HTTP for the session `raw`: the device token.
async fn signed_in_device(state: &AppState, app: &Router, raw: &str) -> String {
    let started = start(app).await;
    let res = approve(state, app, raw, &code_of(&started, "user_code")).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = poll(app, &code_of(&started, "device_code")).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    code_of(&res.body, "access_token")
}

async fn devices(state: &AppState, app: &Router, raw: &str) -> Value {
    let res = send(
        app,
        as_session(state, raw, Method::GET, "/api/me/devices", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

fn match_body() -> Value {
    json!({
        "format": Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::One,
            deciding: DecidingSet::FullSet,
        }),
        "start": Start {
            first_server: Player { team: Team::A, index: 0 },
            other_first_server: 0,
        },
        "teams": { "a": ["Ana", "Rui"], "b": ["João", "Marta"] },
    })
}

// The flow.

#[tokio::test]
async fn device_flow_end_to_end() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let started = start(&app).await;
    let user_code = code_of(&started, "user_code");
    let device_code = code_of(&started, "device_code");
    assert_eq!(started["verification_uri"], json!("/link"));
    assert_eq!(started["expires_in"], json!(600));
    assert_eq!(started["interval"], json!(5));
    let shape = user_code.split('-').map(str::len).collect::<Vec<_>>();
    assert_eq!(shape, [4, 4], "{user_code}");
    assert!(device_code.len() >= 32);

    assert_poll_error(&poll(&app, &device_code).await, "authorization_pending");

    let res = lookup(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["device_name"], json!("Galaxy Watch6"));
    assert!(res.body["created_secs_ago"].as_i64().unwrap() >= 0);
    let left = res.body["expires_in_secs"].as_i64().unwrap();
    assert!((590..=600).contains(&left), "{left}");

    let res = approve(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);

    let res = poll(&app, &device_code).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["token_type"], json!("bearer"));
    let token = code_of(&res.body, "access_token");
    assert!(token.starts_with("pdl_"));
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));

    let res = send(&app, as_device(&token, Method::GET, "/api/auth/me", None)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["id"], json!(admin_id));
    let res = send(
        &app,
        as_device(&token, Method::GET, "/api/me/matches", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!([]));
}

#[tokio::test]
async fn device_name_is_trimmed_and_defaulted() {
    let (state, _, app, _, admin) = with_admin().await;
    let long = "x".repeat(60);
    let cases = [
        (
            json!({ "device_name": "  SM-R960  " }),
            "SM-R960".to_owned(),
        ),
        (json!({}), "Watch".to_owned()),
        (json!({ "device_name": "   " }), "Watch".to_owned()),
        (json!({ "device_name": long }), "x".repeat(40)),
    ];
    for (body, want) in cases {
        let started = start_with(&app, body).await;
        let res = lookup(&state, &app, &admin, &code_of(&started, "user_code")).await;
        assert_eq!(res.body["device_name"], json!(want));
    }
}

#[tokio::test]
async fn token_issued_exactly_once() {
    let (state, _, app, _, admin) = with_admin().await;
    let started = start(&app).await;
    let device_code = code_of(&started, "device_code");
    let res = approve(&state, &app, &admin, &code_of(&started, "user_code")).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    let polls = (0..5).map(|_| poll(&app, &device_code));
    let results = futures_util::future::join_all(polls).await;
    let issued = results
        .iter()
        .filter(|r| r.status == StatusCode::OK)
        .count();
    assert_eq!(issued, 1, "the token is issued exactly once");
    for res in results.iter().filter(|r| r.status != StatusCode::OK) {
        assert_poll_error(res, "expired_token");
    }
    assert_poll_error(&poll(&app, &device_code).await, "expired_token");
}

#[tokio::test]
async fn poll_too_fast_is_slow_down() {
    let (_, pool, app, _, _) = with_admin().await;
    let started = start(&app).await;
    let device_code = code_of(&started, "device_code");
    assert_poll_error(&poll(&app, &device_code).await, "authorization_pending");
    assert_poll_error(&poll(&app, &device_code).await, "slow_down");
    // Waiting the interval out is pending again.
    sqlx::query("UPDATE device_codes SET last_polled_at = last_polled_at - 5")
        .execute(&pool)
        .await
        .unwrap();
    assert_poll_error(&poll(&app, &device_code).await, "authorization_pending");
}

#[tokio::test]
async fn expired_code_is_expired_token() {
    let (state, pool, app, _, admin) = with_admin().await;
    let started = start(&app).await;
    let user_code = code_of(&started, "user_code");
    sqlx::query("UPDATE device_codes SET expires_at = expires_at - 601")
        .execute(&pool)
        .await
        .unwrap();
    assert_poll_error(
        &poll(&app, &code_of(&started, "device_code")).await,
        "expired_token",
    );
    let res = lookup(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = approve(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body, json!({ "error": "not_found" }));
}

#[tokio::test]
async fn unknown_code_is_expired_token() {
    let (_, pool, app, _, _) = with_admin().await;
    assert_poll_error(&poll(&app, "no-such-device-code").await, "expired_token");
    // A purged code reads the same.
    let started = start(&app).await;
    sqlx::query("DELETE FROM device_codes")
        .execute(&pool)
        .await
        .unwrap();
    assert_poll_error(
        &poll(&app, &code_of(&started, "device_code")).await,
        "expired_token",
    );
}

#[tokio::test]
async fn denied_is_access_denied() {
    let (state, _, app, _, admin) = with_admin().await;
    let started = start(&app).await;
    let res = decide(
        &state,
        &app,
        &admin,
        &code_of(&started, "user_code"),
        "deny",
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert_poll_error(
        &poll(&app, &code_of(&started, "device_code")).await,
        "access_denied",
    );
}

#[tokio::test]
async fn approve_requires_session_not_device_token() {
    let (state, _, app, _, admin) = with_admin().await;
    let token = signed_in_device(&state, &app, &admin).await;
    let started = start(&app).await;
    let user_code = code_of(&started, "user_code");
    let lookup_path = format!("/api/device/{user_code}");
    for verb in ["approve", "deny"] {
        let path = format!("/api/device/{user_code}/{verb}");
        let res = send(&app, as_device(&token, Method::POST, &path, None)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{verb}");
        let res = send(&app, req(Method::POST, &path, None, &[])).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{verb}");
    }
    let res = send(&app, as_device(&token, Method::GET, &lookup_path, None)).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = send(&app, req(Method::GET, &lookup_path, None, &[])).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    // A cross-site approval carrying the cookie is refused too.
    let path = format!("/api/device/{user_code}/approve");
    let request = Request::post(&path)
        .header("sec-fetch-site", "cross-site")
        .header("cookie", cookie(&state, &admin))
        .body(Body::empty())
        .unwrap();
    let res = send(&app, request).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // Device tokens can't manage devices either.
    let res = send(
        &app,
        as_device(&token, Method::GET, "/api/me/devices", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    // Still pending: nothing above decided it.
    let res = lookup(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::OK);
}

#[tokio::test]
async fn user_code_lookup_normalizes() {
    let (state, _, app, _, admin) = with_admin().await;
    let started = start(&app).await;
    let user_code = code_of(&started, "user_code");
    let bare: String = user_code.chars().filter(|c| *c != '-').collect();
    let lower = user_code.to_lowercase();
    let (first, second) = bare.split_at(4);
    let spaced = format!("{}%20{}", first.to_lowercase(), second);
    for typed in [user_code.clone(), lower, bare.clone(), spaced] {
        let res = lookup(&state, &app, &admin, &typed).await;
        assert_eq!(res.status, StatusCode::OK, "{typed}: {}", res.body);
        assert_eq!(res.body["device_name"], json!("Galaxy Watch6"));
    }
    // Garbage is 404, and a query string is never read.
    for path in [
        "/api/device/NOPE".to_owned(),
        format!("/api/device/lookup?user_code={user_code}"),
        format!("/api/device?user_code={user_code}"),
    ] {
        let res = send(&app, as_session(&state, &admin, Method::GET, &path, None)).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    // Approving by the bare lowercase form works as well.
    let res = approve(&state, &app, &admin, &bare.to_lowercase()).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
}

#[tokio::test]
async fn decided_code_is_409() {
    let (state, _, app, _, admin) = with_admin().await;
    let started = start(&app).await;
    let user_code = code_of(&started, "user_code");
    let res = approve(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    for verb in ["approve", "deny"] {
        let res = decide(&state, &app, &admin, &user_code, verb).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{verb}");
        assert_eq!(res.body, json!({ "error": "device_code_used" }));
    }
    // A decided code is no longer shown, nor once consumed.
    let res = lookup(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = poll(&app, &code_of(&started, "device_code")).await;
    assert_eq!(res.status, StatusCode::OK);
    let res = lookup(&state, &app, &admin, &user_code).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    let denied = start(&app).await;
    let code = code_of(&denied, "user_code");
    let res = decide(&state, &app, &admin, &code, "deny").await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = approve(&state, &app, &admin, &code).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    // Unknown is 404, not 409.
    let res = approve(&state, &app, &admin, "ZZZZ-ZZZZ").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn device_start_and_approve_are_rate_limited() {
    let (state, _, app, _, admin) = with_admin().await;
    for _ in 0..10 {
        start(&app).await;
    }
    let res = send(
        &app,
        watch_post("/api/device/start", json!({ "device_name": "W" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    assert!(res.header(header::RETRY_AFTER).is_some());
    // Another address has its own budget.
    start(&app_from(&state, 2)).await;

    // Approvals count every call, found or not, per user.
    let code = code_of(&start(&app_from(&state, 3)).await, "user_code");
    for i in 0..10 {
        let verb = if i % 2 == 0 { "approve" } else { "deny" };
        let res = decide(&state, &app, &admin, "ZZZZ-ZZZZ", verb).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    let res = approve(&state, &app, &admin, &code).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(res.header(header::RETRY_AFTER).is_some());
    let res = decide(&state, &app, &admin, &code, "deny").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    // Another user is not affected.
    let (_, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let res = approve(&state, &app, &bea, &code).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
}

// Device management.

#[tokio::test]
async fn devices_list_and_revoke_own_only() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let admin_token = signed_in_device(&state, &app, &admin).await;
    let bea_token = signed_in_device(&state, &app, &bea).await;

    let list = devices(&state, &app, &admin).await;
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 1);
    let mine = &list[0];
    assert_eq!(mine["name"], json!("Galaxy Watch6"));
    assert_eq!(
        mine["prefix"],
        json!(admin_token.chars().take(8).collect::<String>())
    );
    assert!(mine["created_at"].is_i64());
    assert!(mine.get("last_used_at").is_none(), "{mine}");
    assert!(!mine.to_string().contains(&admin_token));
    let admin_device = mine["id"].as_str().unwrap().to_owned();

    // Using the token shows up.
    let res = send(
        &app,
        as_device(&admin_token, Method::GET, "/api/me/matches", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let list = devices(&state, &app, &admin).await;
    assert!(list[0]["last_used_at"].is_i64(), "{list}");

    let bea_list = devices(&state, &app, &bea).await;
    let bea_device = bea_list[0]["id"].as_str().unwrap().to_owned();
    assert_ne!(bea_device, admin_device);

    // Someone else's device is 404, and stays signed in.
    let path = format!("/api/me/devices/{bea_device}");
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = send(
        &app,
        as_device(&bea_token, Method::GET, "/api/me/matches", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);

    // One's own is revoked.
    let res = send(&app, as_session(&state, &bea, Method::DELETE, &path, None)).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert_eq!(devices(&state, &app, &bea).await, json!([]));
    let res = send(
        &app,
        as_device(&bea_token, Method::GET, "/api/me/matches", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    // Again is 404.
    let res = send(&app, as_session(&state, &bea, Method::DELETE, &path, None)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // The admin's device is untouched.
    assert_eq!(
        devices(&state, &app, &admin)
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn revoked_device_stops_streams_on_recheck() {
    let (mut state, _, _, _, admin) = with_admin().await;
    state.credential_recheck = Duration::from_millis(50);
    let app = app_from(&state, 1);
    let token = signed_in_device(&state, &app, &admin).await;
    let res = send(
        &app,
        as_session(
            &state,
            &admin,
            Method::POST,
            "/api/matches",
            Some(match_body()),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED);
    let match_path = format!(
        "/api/me/matches/{}/stream",
        res.body["id"].as_str().unwrap()
    );

    let mut list = Sse::open(
        &app,
        as_device(&token, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    list.next("matches").await;
    let mut one = Sse::open(&app, as_device(&token, Method::GET, &match_path, None)).await;
    one.next("state").await;
    assert!(!list.ends_within(Duration::from_millis(200)).await);

    let id = devices(&state, &app, &admin).await[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/api/me/devices/{id}");
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    assert!(
        list.ends_within(SSE_TIMEOUT).await,
        "list stream outlived its device"
    );
    assert!(
        one.ends_within(SSE_TIMEOUT).await,
        "match stream outlived its device"
    );
    for path in ["/api/me/matches/stream", match_path.as_str()] {
        let res = send(&app, as_device(&token, Method::GET, path, None)).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn device_name_strips_control_and_format_chars() {
    let (state, _, app, _, admin) = with_admin().await;
    let cases = [
        // Bidi override, zero-width space, BOM, soft hyphen and a control char are all dropped.
        (
            "\u{202E}Gal\u{200B}axy\u{FEFF} W\u{AD}atch\u{7}",
            "Galaxy Watch",
        ),
        ("\u{200B}\u{202E}\n\t", "Watch"),
        ("  \u{2066}Pixel\u{2069}  ", "Pixel"),
        ("Wätch é", "Wätch é"),
    ];
    for (given, want) in cases {
        let started = start_with(&app, json!({ "device_name": given })).await;
        let res = lookup(&state, &app, &admin, &code_of(&started, "user_code")).await;
        assert_eq!(res.body["device_name"], json!(want), "for {given:?}");
    }
}

#[tokio::test]
async fn device_lookup_is_rate_limited() {
    let (state, _, app, _, admin) = with_admin().await;
    for _ in 0..30 {
        let res = lookup(&state, &app, &admin, "ZZZZ-ZZZZ").await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    let res = lookup(&state, &app, &admin, "ZZZZ-ZZZZ").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    assert!(res.header(header::RETRY_AFTER).is_some());
    // Another user has their own budget.
    let (_, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let res = lookup(&state, &app, &bea, "ZZZZ-ZZZZ").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn device_token_route_is_rate_limited_per_ip() {
    let (state, _, app, _, _) = with_admin().await;
    for _ in 0..60 {
        assert_poll_error(&poll(&app, "unknown").await, "expired_token");
    }
    let res = poll(&app, "unknown").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    assert!(res.header(header::RETRY_AFTER).is_some());
    // Another address has its own budget.
    assert_poll_error(
        &poll(&app_from(&state, 2), "unknown").await,
        "expired_token",
    );
}

#[tokio::test]
async fn unknown_device_code_skips_write_lock() {
    let (state, _, app, _, _) = with_admin().await;
    // Behaviour: unknown is expired_token and leaves no trace.
    assert_poll_error(&poll(&app, "no-such-code").await, "expired_token");
    // The read-only existence check the route uses first.
    assert!(
        !state
            .store
            .device_code_exists("no-such-code")
            .await
            .unwrap()
    );
    let started = start(&app).await;
    assert!(
        state
            .store
            .device_code_exists(&code_of(&started, "device_code"))
            .await
            .unwrap()
    );
}
