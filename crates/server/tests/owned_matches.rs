#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Owned matches: ownership on create, the live list and its stream, and the watch's routes
//! (`/api/me/matches/{id}/commands` and `/stream`), through the real router.

mod support;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use padel_score_server::AppState;
use padel_score_server::api::now;
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use support::*;
use tower::ServiceExt;

const TIMEOUT: Duration = Duration::from_secs(2);

fn match_body(a: [&str; 2], b: [&str; 2]) -> Value {
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
        "teams": { "a": a, "b": b },
    })
}

fn default_body() -> Value {
    match_body(["Ana", "Rui"], ["João", "Marta"])
}

struct Created {
    id: String,
    scorer: String,
    spectator: String,
}

fn created(res: &Res) -> Created {
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    Created {
        id: res.body["id"].as_str().unwrap().to_owned(),
        scorer: res.body["scorer_token"].as_str().unwrap().to_owned(),
        spectator: res.body["spectator_token"].as_str().unwrap().to_owned(),
    }
}

/// A match created signed in by the session `raw`.
async fn create_as(state: &AppState, app: &Router, raw: &str, body: Value) -> Created {
    let res = send(
        app,
        as_session(state, raw, Method::POST, "/api/matches", Some(body)),
    )
    .await;
    created(&res)
}

async fn create_anon(app: &Router) -> Created {
    let res = send(
        app,
        req(Method::POST, "/api/matches", Some(default_body()), &[]),
    )
    .await;
    created(&res)
}

fn point(client_id: &str, team: &str, n: usize) -> Value {
    let commands: Vec<Value> = (0..n)
        .map(|_| json!({ "kind": "point", "team": team }))
        .collect();
    json!({ "client_id": client_id, "batch": false, "base_revision": 0, "commands": commands })
}

/// Score through the phase 2 scorer-token route.
async fn score_by_token(app: &Router, scorer: &str, body: Value) -> Res {
    let path = format!("/api/score/{scorer}/commands");
    send(app, req(Method::POST, &path, Some(body), &[])).await
}

fn owner_commands_path(id: &str) -> String {
    format!("/api/me/matches/{id}/commands")
}

async fn list_by_session(state: &AppState, app: &Router, raw: &str) -> Res {
    send(
        app,
        as_session(state, raw, Method::GET, "/api/me/matches", None),
    )
    .await
}

async fn list_by_device(app: &Router, token: &str) -> Res {
    send(app, as_device(token, Method::GET, "/api/me/matches", None)).await
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect()
}

async fn user_and_device(state: &AppState, app: &Router, admin: &str) -> (String, String, String) {
    let (id, raw) = invited_user(state, app, admin, "bea@x.test", "Bea").await;
    let token = device_token(&state.store, &id).await;
    (id, raw, token)
}

// Ownership on create.

#[tokio::test]
async fn signed_in_create_sets_owner() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let owned = state.store.match_owned_by(&m.id, &admin_id).await.unwrap();
    assert_eq!(owned.unwrap().owner_user_id.as_deref(), Some(&*admin_id));
}

#[tokio::test]
async fn anonymous_create_unchanged() {
    let (state, _, app, _, _) = with_admin().await;
    let res = send(
        &app,
        req(Method::POST, "/api/matches", Some(default_body()), &[]),
    )
    .await;
    let m = created(&res);
    let keys: Vec<&String> = res.body.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "scorer_token", "spectator_token"]);
    assert_eq!(
        res.header(header::LOCATION),
        Some(&*format!("/s/{}", m.scorer))
    );
    let stored = state.store.by_scorer_token(&m.scorer).await.unwrap();
    assert_eq!(stored.unwrap().owner_user_id, None);
}

#[tokio::test]
async fn bad_cookie_create_is_anonymous_not_401() {
    let (state, _, app, _, _) = with_admin().await;
    let bad_cookie = cookie(&state, "not-a-session");
    let requests = [
        req(
            Method::POST,
            "/api/matches",
            Some(default_body()),
            &[("cookie", &bad_cookie)],
        ),
        as_device(
            "pdl_not_a_token",
            Method::POST,
            "/api/matches",
            Some(default_body()),
        ),
    ];
    for request in requests {
        let res = send(&app, request).await;
        let m = created(&res);
        let stored = state.store.by_scorer_token(&m.scorer).await.unwrap();
        assert_eq!(stored.unwrap().owner_user_id, None);
    }
}

// The list.

#[tokio::test]
async fn me_matches_lists_only_own_live_newest_first() {
    let (state, pool, app, _, admin) = with_admin().await;
    let (_, bea, _) = user_and_device(&state, &app, &admin).await;
    let older = create_as(&state, &app, &admin, default_body()).await;
    let newer = create_as(
        &state,
        &app,
        &admin,
        match_body(["Eva", "Ivo"], ["Leo", "Mia"]),
    )
    .await;
    let beas = create_as(&state, &app, &bea, default_body()).await;
    let anon = create_anon(&app).await;
    // The older match was scored most recently, so it comes first.
    let t = now();
    for (id, at) in [(&older.id, t + 20), (&newer.id, t + 10)] {
        sqlx::query("UPDATE matches SET last_activity_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }

    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(ids(&res.body), [older.id.clone(), newer.id.clone()]);
    let first = &res.body[0];
    assert_eq!(first["label"], json!("Ana / Rui v João / Marta"));
    assert_eq!(res.body[1]["label"], json!("Eva / Ivo v Leo / Mia"));
    assert_eq!(first["last_activity_at"], json!(t + 20));
    assert!(first["created_at"].as_i64().unwrap() <= t);
    assert_eq!(first["view"]["id"], json!(older.id));
    assert_eq!(first["view"]["revision"], json!(0));
    assert_eq!(
        first["view"]["teams"],
        json!({ "a": ["Ana", "Rui"], "b": ["João", "Marta"] })
    );

    let res = list_by_session(&state, &app, &bea).await;
    assert_eq!(ids(&res.body), [beas.id]);
    assert!(!res.body.to_string().contains(&anon.id));

    // No credential at all.
    let res = send(&app, req(Method::GET, "/api/me/matches", None, &[])).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn finished_matches_drop_out() {
    let (state, _, app, _, admin) = with_admin().await;
    let done = create_as(&state, &app, &admin, default_body()).await;
    let live = create_as(&state, &app, &admin, default_body()).await;
    // One set to six: 24 straight points finish it.
    let res = score_by_token(&app, &done.scorer, point("finish", "a", 24)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body["score"]["status"],
        json!({ "finished": { "winner": "a" } })
    );

    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(ids(&res.body), [live.id]);
}

#[tokio::test]
async fn list_keeps_at_most_twenty() {
    let (state, _, app, _, admin) = with_admin().await;
    for _ in 0..22 {
        create_as(&state, &app, &admin, default_body()).await;
    }
    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.body.as_array().unwrap().len(), 20);
}

#[tokio::test]
async fn device_caller_gets_no_tokens() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let res = list_by_device(&app, &token).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(ids(&res.body), [m.id]);
    let item = res.body[0].as_object().unwrap();
    assert!(!item.contains_key("scorer_token"), "{item:?}");
    assert!(!item.contains_key("spectator_token"), "{item:?}");
    let text = res.body.to_string();
    assert!(!text.contains(&m.scorer) && !text.contains(&m.spectator));
}

#[tokio::test]
async fn session_caller_gets_tokens() {
    let (state, _, app, _, admin) = with_admin().await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.body[0]["scorer_token"], json!(m.scorer));
    assert_eq!(res.body[0]["spectator_token"], json!(m.spectator));
}

// The list stream.

#[tokio::test]
async fn list_stream_sends_on_create_and_on_score() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, bea, _) = user_and_device(&state, &app, &admin).await;
    let mut stream = Sse::open(
        &app,
        as_session(&state, &admin, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    assert_eq!(stream.next("matches").await, json!([]));

    let m = create_as(&state, &app, &admin, default_body()).await;
    let list = stream.next("matches").await;
    assert_eq!(ids(&list), std::slice::from_ref(&m.id));
    assert_eq!(list[0]["label"], json!("Ana / Rui v João / Marta"));
    assert_eq!(list[0]["scorer_token"], json!(m.scorer));
    assert_eq!(list[0]["view"]["revision"], json!(0));

    // Another user's match is not this stream's business: the next frame is the point.
    create_as(&state, &app, &bea, default_body()).await;
    let res = score_by_token(&app, &m.scorer, point("p1", "a", 1)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let list = stream.next("matches").await;
    assert_eq!(ids(&list), std::slice::from_ref(&m.id));
    assert_eq!(list[0]["view"]["revision"], json!(1));

    // Finishing it takes it off the list at once.
    let res = score_by_token(&app, &m.scorer, point("fin", "a", 23)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(stream.next("matches").await, json!([]));
}

#[tokio::test]
async fn list_stream_by_device_has_no_tokens() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let mut stream = Sse::open(
        &app,
        as_device(&token, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    let list = stream.next("matches").await;
    assert_eq!(ids(&list), std::slice::from_ref(&m.id));
    assert!(!list.to_string().contains(&m.scorer));
}

#[tokio::test]
async fn list_stream_ends_when_device_revoked() {
    let (mut state, _, _, admin_id, _) = with_admin().await;
    state.credential_recheck = Duration::from_millis(50);
    let app = app_from(&state, 1);
    let token = device_token(&state.store, &admin_id).await;
    let mut list = Sse::open(
        &app,
        as_device(&token, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    assert_eq!(list.next("matches").await, json!([]));
    // Still open while the token is good.
    assert!(!list.ends_within(Duration::from_millis(200)).await);

    let devices = state.store.list_devices(&admin_id).await.unwrap();
    state
        .store
        .revoke_device(&admin_id, &devices[0].id, now())
        .await
        .unwrap();
    assert!(list.ends_within(TIMEOUT).await, "stream outlived its token");
}

// The watch's routes.

#[tokio::test]
async fn owner_command_route_scores_and_publishes() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let fence_path = format!("/api/view/{}/stream", m.spectator);
    let mut fence = Sse::open(&app, req(Method::GET, &fence_path, None, &[])).await;
    assert_eq!(fence.next("state").await["revision"], json!(0));
    let mut list = Sse::open(
        &app,
        as_device(&token, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    assert_eq!(list.next("matches").await[0]["view"]["revision"], json!(0));

    let path = owner_commands_path(&m.id);
    let res = send(
        &app,
        as_device(&token, Method::POST, &path, Some(point("w1", "a", 1))),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["revision"], json!(1));
    assert_eq!(res.header(header::CONTENT_TYPE), Some("application/json"));
    let seen = fence.next("state").await;
    assert_eq!(seen, res.body);
    assert_eq!(list.next("matches").await[0]["view"], res.body);

    // The session works too, and the same shapes come back as on the scorer route.
    let res = send(
        &app,
        as_session(
            &state,
            &admin,
            Method::POST,
            &path,
            Some(point("s1", "b", 1)),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(fence.next("state").await["revision"], json!(2));
    let stale = json!({
        "client_id": "batch1", "batch": true, "base_revision": 0,
        "commands": [{ "kind": "point", "team": "a" }],
    });
    let res = send(&app, as_device(&token, Method::POST, &path, Some(stale))).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("conflict"));
    assert_eq!(res.body["view"]["revision"], json!(2));
    // A malformed body is the same 400 as anywhere.
    let res = send(
        &app,
        as_device(&token, Method::POST, &path, Some(json!({}))),
    )
    .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn owner_command_route_refuses_cross_site_cookie() {
    let (state, _, app, _, admin) = with_admin().await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let request = Request::post(owner_commands_path(&m.id))
        .header("sec-fetch-site", "cross-site")
        .header("cookie", cookie(&state, &admin))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(point("x1", "a", 1).to_string()))
        .unwrap();
    let res = send(&app, request).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let view = list_by_session(&state, &app, &admin).await;
    assert_eq!(view.body[0]["view"]["revision"], json!(0));
}

#[tokio::test]
async fn other_user_cannot_see_stream_or_score() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, bea, bea_token) = user_and_device(&state, &app, &admin).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let anon = create_anon(&app).await;

    let res = list_by_session(&state, &app, &bea).await;
    assert_eq!(res.body, json!([]));
    let res = list_by_device(&app, &bea_token).await;
    assert_eq!(res.body, json!([]));

    for id in [m.id.as_str(), anon.id.as_str(), "no-such-match"] {
        let commands = owner_commands_path(id);
        let stream = format!("/api/me/matches/{id}/stream");
        let requests = [
            as_device(
                &bea_token,
                Method::POST,
                &commands,
                Some(point("b1", "a", 1)),
            ),
            as_session(
                &state,
                &bea,
                Method::POST,
                &commands,
                Some(point("b2", "a", 1)),
            ),
            as_device(&bea_token, Method::GET, &stream, None),
            as_session(&state, &bea, Method::GET, &stream, None),
        ];
        for request in requests {
            let res = send(&app, request).await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{id}");
            assert_eq!(res.body, json!({ "error": "not_found" }));
        }
    }
    // Nothing was scored.
    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.body[0]["view"]["revision"], json!(0));
}

#[tokio::test]
async fn revoked_device_gets_401() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let commands = owner_commands_path(&m.id);
    let stream = format!("/api/me/matches/{}/stream", m.id);
    let res = send(
        &app,
        as_device(&token, Method::POST, &commands, Some(point("d1", "a", 1))),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);

    let devices = state.store.list_devices(&admin_id).await.unwrap();
    state
        .store
        .revoke_device(&admin_id, &devices[0].id, now())
        .await
        .unwrap();
    let requests = [
        as_device(&token, Method::POST, &commands, Some(point("d2", "a", 1))),
        as_device(&token, Method::GET, &stream, None),
        as_device(&token, Method::GET, "/api/me/matches", None),
        as_device(&token, Method::GET, "/api/me/matches/stream", None),
    ];
    for request in requests {
        let res = send(&app, request).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        assert_eq!(res.body, json!({ "error": "unauthorized" }));
    }
    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.body[0]["view"]["revision"], json!(1));
}

#[tokio::test]
async fn disabled_users_device_gets_401() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, bea, bea_token) = user_and_device(&state, &app, &admin).await;
    let m = create_as(&state, &app, &bea, default_body()).await;
    let path = format!("/api/admin/users/{bea_id}/disable");
    let res = send(&app, as_session(&state, &admin, Method::POST, &path, None)).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let stream = format!("/api/me/matches/{}/stream", m.id);
    let res = send(&app, as_device(&bea_token, Method::GET, &stream, None)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn per_match_stream_first_frame_then_next_after_point() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let path = format!("/api/me/matches/{}/stream", m.id);
    let mut stream = Sse::open(&app, as_device(&token, Method::GET, &path, None)).await;
    let first = stream.next("state").await;
    assert_eq!(first["id"], json!(m.id));
    assert_eq!(first["revision"], json!(0));
    // The same frame the scorer's stream sends.
    let res = send(
        &app,
        req(Method::GET, &format!("/api/score/{}", m.scorer), None, &[]),
    )
    .await;
    assert_eq!(first, res.body);
    assert!(!first.to_string().contains(&m.scorer));

    let res = send(
        &app,
        as_device(
            &token,
            Method::POST,
            &owner_commands_path(&m.id),
            Some(point("p", "b", 1)),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let next = stream.next("state").await;
    assert_eq!(next["revision"], json!(1));
    assert_eq!(next, res.body);
}

#[tokio::test]
async fn per_match_stream_ends_when_device_revoked() {
    let (mut state, _, _, admin_id, admin) = with_admin().await;
    state.credential_recheck = Duration::from_millis(50);
    let app = app_from(&state, 1);
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let path = format!("/api/me/matches/{}/stream", m.id);
    let mut stream = Sse::open(&app, as_device(&token, Method::GET, &path, None)).await;
    assert_eq!(stream.next("state").await["revision"], json!(0));
    let devices = state.store.list_devices(&admin_id).await.unwrap();
    state
        .store
        .revoke_device(&admin_id, &devices[0].id, now())
        .await
        .unwrap();
    assert!(
        stream.ends_within(TIMEOUT).await,
        "stream outlived its token"
    );
}

#[tokio::test]
async fn owner_command_idempotent_with_client_id() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let m = create_as(&state, &app, &admin, default_body()).await;
    let path = owner_commands_path(&m.id);
    let send_raw = async |request: Request<Body>| {
        let res = app.clone().oneshot(request).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, bytes)
    };
    let first = send_raw(as_device(
        &token,
        Method::POST,
        &path,
        Some(point("same", "a", 1)),
    ))
    .await;
    let again = send_raw(as_device(
        &token,
        Method::POST,
        &path,
        Some(point("same", "a", 1)),
    ))
    .await;
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first, again, "a resend is the first answer, byte for byte");
    // The idempotency is the match's, shared with the scorer route.
    let by_token = send_raw(req(
        Method::POST,
        &format!("/api/score/{}/commands", m.scorer),
        Some(point("same", "a", 1)),
        &[],
    ))
    .await;
    assert_eq!(first, by_token);
    let res = list_by_session(&state, &app, &admin).await;
    assert_eq!(res.body[0]["view"]["revision"], json!(1));
}

#[tokio::test]
async fn hub_drops_user_entry_when_list_stream_goes() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let mut stream = Sse::open(
        &app,
        as_session(&state, &admin, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    assert_eq!(stream.next("matches").await, json!([]));
    assert!(state.hub.is_user_watched(&admin_id));
    drop(stream);
    assert!(!state.hub.is_user_watched(&admin_id));
}

// Stream hardening.

/// The pool's only connection is held while the list re-read starts, so the re-read parks
/// waiting for it; then the reader stalls (its body is never polled again). Once the hold is
/// dropped, the stalled stream must not end up pinning the connection.
#[tokio::test]
async fn stalled_reread_does_not_hold_pool_connection() {
    let (state, pool, app, admin_id, admin) = with_admin().await;
    create_as(&state, &app, &admin, default_body()).await;
    let mut stream = Sse::open(
        &app,
        as_session(&state, &admin, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    assert_eq!(ids(&stream.next("matches").await).len(), 1);

    let held = pool.acquire().await.unwrap();
    state.hub.notify_user(&admin_id);
    // One poll starts the re-read, which waits for the held connection.
    assert!(
        tokio::time::timeout(Duration::from_millis(100), stream.body.frame())
            .await
            .is_err()
    );
    drop(held);
    // The stream is never polled again, yet the pool is free for everyone else.
    let free = tokio::time::timeout(TIMEOUT, pool.acquire()).await;
    assert!(free.is_ok(), "a stalled list stream pinned the pool");
    drop(free);
    drop(stream);
}

/// The same for the phase 2 per-match stream's re-read after it lags.
#[tokio::test]
async fn stalled_lag_reread_does_not_hold_pool_connection() {
    let (_, pool, app, _, _) = with_admin().await;
    let m = create_anon(&app).await;
    let path = format!("/api/view/{}/stream", m.spectator);
    let mut stream = Sse::open(&app, req(Method::GET, &path, None, &[])).await;
    assert_eq!(stream.next("state").await["revision"], json!(0));
    // More publishes than the channel holds, unread: the stream lags.
    for i in 0..20 {
        let res = score_by_token(&app, &m.scorer, point(&format!("p{i}"), "a", 1)).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }
    let held = pool.acquire().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), stream.body.frame())
            .await
            .is_err()
    );
    drop(held);
    let free = tokio::time::timeout(TIMEOUT, pool.acquire()).await;
    assert!(free.is_ok(), "a stalled match stream pinned the pool");
    drop(free);
    drop(stream);
}

#[tokio::test]
async fn sixth_list_stream_evicts_oldest() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, bea, _) = user_and_device(&state, &app, &admin).await;
    let open = || as_session(&state, &admin, Method::GET, "/api/me/matches/stream", None);
    let mut streams = Vec::new();
    for _ in 0..5 {
        let mut s = Sse::open(&app, open()).await;
        s.next("matches").await;
        streams.push(s);
    }
    // A 6th is admitted, and the oldest ends to make room.
    let mut sixth = Sse::open(&app, open()).await;
    sixth.next("matches").await;
    let mut oldest = streams.remove(0);
    assert!(
        oldest.ends_within(TIMEOUT).await,
        "the oldest stream lives on"
    );
    // The other four live on: never more than five.
    for s in &mut streams {
        assert!(!s.ends_within(Duration::from_millis(50)).await);
    }
    // A 7th evicts the next oldest, and so on.
    let mut seventh = Sse::open(&app, open()).await;
    seventh.next("matches").await;
    assert!(streams[0].ends_within(TIMEOUT).await);
    assert!(!sixth.ends_within(Duration::from_millis(50)).await);
    // Another user has their own allowance; their streams are not evicted.
    let mut other = Sse::open(
        &app,
        as_session(&state, &bea, Method::GET, "/api/me/matches/stream", None),
    )
    .await;
    other.next("matches").await;
    assert!(!streams[1].ends_within(Duration::from_millis(50)).await);
    // A closed stream frees its slot: opening after it evicts nobody.
    drop(seventh);
    let mut again = Sse::open(&app, open()).await;
    again.next("matches").await;
    for s in &mut streams[1..] {
        assert!(!s.ends_within(Duration::from_millis(50)).await);
    }
    assert!(!sixth.ends_within(Duration::from_millis(50)).await);
}

/// The list carries scorer tokens: no cache may keep it.
#[tokio::test]
async fn owned_routes_are_no_store() {
    let (state, _, app, _, admin) = with_admin().await;
    let res = send(
        &app,
        as_session(&state, &admin, Method::GET, "/api/me/matches", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    assert_eq!(res.header(header::REFERRER_POLICY), Some("no-referrer"));
    // Errors too.
    let res = send(&app, req(Method::GET, "/api/me/matches", None, &[])).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
}
