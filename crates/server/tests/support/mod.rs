//! Helpers shared by the account route tests (invites, admin): a router on an in-memory
//! database whose raw pool the test keeps too, requests as the PWA sends them, and users made
//! through the API.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use padel_score_server::accounts::init_setup_code;
use padel_score_server::accounts_store::PollOutcome;
use padel_score_server::api::now;
use padel_score_server::auth::cookie_name;
use padel_score_server::{AppState, Config, Store, db, router};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery";
pub const ADMIN: &str = "admin@x.test";

/// A state on a fresh in-memory database, and the same pool for raw SQL.
pub async fn state() -> (AppState, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let config = Config::from_map(&HashMap::new()).unwrap();
    (
        AppState::new(Arc::new(config), Store::new(pool.clone())),
        pool,
    )
}

pub fn peer(last: u8) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, last], 40_000))
}

/// The router as served, with the TCP peer `203.0.113.{last}`.
pub fn app_from(state: &AppState, last: u8) -> Router {
    router(state.clone()).layer(MockConnectInfo(peer(last)))
}

pub struct Res {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Res {
    /// The session value set by this response's `Set-Cookie`, `""` when it clears it.
    pub fn session(&self, state: &AppState) -> Option<String> {
        let prefix = format!("{}=", cookie_name(&state.config));
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.strip_prefix(&prefix))
            .map(|rest| rest.split(';').next().unwrap_or("").to_owned())
    }

    pub fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

pub async fn send(app: &Router, req: Request<Body>) -> Res {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)))
    };
    Res {
        status,
        headers,
        body,
    }
}

/// A same-origin request (as the PWA sends it), with an optional JSON body and extra headers.
pub fn req(
    method: Method,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("sec-fetch-site", "same-origin");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    match body {
        Some(body) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

/// The `Cookie` header value carrying the session `raw`.
pub fn cookie(state: &AppState, raw: &str) -> String {
    format!("{}={raw}", cookie_name(&state.config))
}

/// `req` signed in by the session `raw`.
pub fn as_session(
    state: &AppState,
    raw: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Request<Body> {
    req(method, path, body, &[("cookie", &cookie(state, raw))])
}

/// `req` signed in by the device token `token`.
pub fn as_device(token: &str, method: Method, path: &str, body: Option<Value>) -> Request<Body> {
    let bearer = format!("Bearer {token}");
    req(method, path, body, &[("authorization", &bearer)])
}

pub async fn login(app: &Router, email: &str, password: &str) -> Res {
    send(
        app,
        req(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
            &[],
        ),
    )
    .await
}

pub async fn me(state: &AppState, app: &Router, raw: &str) -> Res {
    send(
        app,
        as_session(state, raw, Method::GET, "/api/auth/me", None),
    )
    .await
}

/// The first admin, made through the API: (state, pool, app, admin id, admin session raw).
pub async fn with_admin() -> (AppState, SqlitePool, Router, String, String) {
    let (state, pool) = state().await;
    let code = init_setup_code(&state).await.unwrap().unwrap();
    let app = app_from(&state, 1);
    let body = json!({
        "setup_code": code, "email": ADMIN, "display_name": "Ana", "password": PASSWORD,
    });
    let res = send(&app, req(Method::POST, "/api/setup", Some(body), &[])).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let raw = res.session(&state).unwrap();
    let id = res.body["id"].as_str().unwrap().to_owned();
    (state, pool, app, id, raw)
}

/// The code in an `/invite#<code>` or `/reset#<code>` URL.
pub fn fragment_code(url: &str) -> String {
    url.split_once('#').unwrap().1.to_owned()
}

/// A new invite by the session `raw`: its (id, code).
pub async fn create_invite(state: &AppState, app: &Router, raw: &str) -> (String, String) {
    let res = send(
        app,
        as_session(state, raw, Method::POST, "/api/invites", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let url = res.body["url"].as_str().unwrap();
    (
        res.body["id"].as_str().unwrap().to_owned(),
        fragment_code(url),
    )
}

pub fn accept_body(email: &str, name: &str) -> Value {
    json!({ "email": email, "display_name": name, "password": PASSWORD })
}

/// `POST /api/invites/accept` with `body` and the code added to it.
pub async fn accept(app: &Router, code: &str, mut body: Value) -> Res {
    body["code"] = json!(code);
    send(
        app,
        req(Method::POST, "/api/invites/accept", Some(body), &[]),
    )
    .await
}

/// A user invited by the session `inviter`, signed in: (user id, session raw).
pub async fn invited_user(
    state: &AppState,
    app: &Router,
    inviter: &str,
    email: &str,
    name: &str,
) -> (String, String) {
    let (_, code) = create_invite(state, app, inviter).await;
    let res = accept(app, &code, accept_body(email, name)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    (
        res.body["id"].as_str().unwrap().to_owned(),
        res.session(state).unwrap(),
    )
}

/// A device token for `user_id`, through the device-code flow in the store.
pub async fn device_token(s: &Store, user_id: &str) -> String {
    let started = s.start_device("Watch", now()).await.unwrap();
    s.approve_device(&started.user_code, user_id, now())
        .await
        .unwrap();
    match s.poll_device(&started.device_code, now()).await.unwrap() {
        PollOutcome::Issued { raw_token } => raw_token,
        _ => panic!("expected a token"),
    }
}

/// How long [`Sse`] waits for a frame.
pub const SSE_TIMEOUT: Duration = Duration::from_secs(2);

/// An open SSE response body, read one named event at a time.
pub struct Sse {
    pub body: Body,
    pub buf: String,
}

impl Sse {
    pub async fn open(app: &Router, request: Request<Body>) -> Sse {
        let res = app.clone().oneshot(request).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        Sse {
            body: res.into_body(),
            buf: String::new(),
        }
    }

    /// The next chunk, or `None` once the body has ended.
    pub async fn chunk(&mut self) -> Option<String> {
        loop {
            let frame = tokio::time::timeout(SSE_TIMEOUT, self.body.frame())
                .await
                .expect("timed out waiting for a frame")?
                .unwrap();
            if let Ok(data) = frame.into_data() {
                return Some(String::from_utf8(data.to_vec()).unwrap());
            }
        }
    }

    /// The next `name` event's data, skipping keep-alives and other events.
    pub async fn next(&mut self, name: &str) -> Value {
        loop {
            if let Some(end) = self.buf.find("\n\n") {
                let event: String = self.buf.drain(..end + 2).collect();
                let mut event_name = None;
                let mut data = String::new();
                for line in event.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        event_name = Some(v.trim().to_owned());
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim_start());
                    }
                }
                if event_name.as_deref() == Some(name) {
                    return serde_json::from_str(&data).unwrap();
                }
                continue;
            }
            let chunk = self.chunk().await.expect("stream ended");
            self.buf.push_str(&chunk);
        }
    }

    /// Whether the body ends within `within`, skipping anything sent before.
    pub async fn ends_within(&mut self, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match tokio::time::timeout_at(deadline, self.body.frame()).await {
                Err(_) => return false,
                Ok(None) => return true,
                Ok(Some(_)) => {}
            }
        }
    }
}

/// The bracket labels of an 8-team tournament with full placement, from the server's own
/// `Layout::label` rather than a copy of its wording.
fn bracket_label(place_from: u32, size: u32, idx: u32) -> String {
    use padel_score_server::competitions::tournament::bracket::{Layout, Pos};
    Layout::new(8, true).unwrap().label(Pos {
        place_from,
        size,
        idx,
    })
}

/// Quarter-final `n` (1-based).
pub fn qf(n: u32) -> String {
    bracket_label(1, 8, n - 1)
}

/// Semi-final `n` (1-based).
pub fn sf(n: u32) -> String {
    bracket_label(1, 4, n - 1)
}

/// Placement semi `n` (1-based), for places 5 to 8.
pub fn ps(n: u32) -> String {
    bracket_label(5, 4, n - 1)
}
