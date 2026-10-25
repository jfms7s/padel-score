#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Setup, sign-in, sign-out, `me` and password change, through the real router.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use padel_score_server::accounts::init_setup_code;
use padel_score_server::accounts_store::{NewUser, PollOutcome};
use padel_score_server::api::now;
use padel_score_server::auth::cookie_name;
use padel_score_server::{AppState, Config, Store, db, router};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
const ADMIN: &str = "admin@x.test";

fn env_map(env: &[(&str, &str)]) -> HashMap<String, String> {
    env.iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

async fn state_on(store: Store, env: &[(&str, &str)]) -> AppState {
    let config = Config::from_map(&env_map(env)).unwrap();
    AppState::new(Arc::new(config), store)
}

async fn state_with(env: &[(&str, &str)]) -> AppState {
    let store = Store::new(db::connect("sqlite::memory:").await.unwrap());
    state_on(store, env).await
}

fn peer(last: u8) -> SocketAddr {
    SocketAddr::from(([203, 0, 113, last], 40_000))
}

/// The router as served, with the TCP peer `203.0.113.{last}`.
fn app_from(state: &AppState, last: u8) -> Router {
    router(state.clone()).layer(MockConnectInfo(peer(last)))
}

struct Res {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Res {
    /// The session value set by this response's `Set-Cookie`, `""` when it clears it.
    fn session(&self, state: &AppState) -> Option<String> {
        let prefix = format!("{}=", cookie_name(&state.config));
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|v| v.strip_prefix(&prefix))
            .map(|rest| rest.split(';').next().unwrap_or("").to_owned())
    }
}

async fn send(app: &Router, req: Request<Body>) -> Res {
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
fn req(method: Method, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Request<Body> {
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

fn cookie(state: &AppState, raw: &str) -> String {
    format!("{}={raw}", cookie_name(&state.config))
}

fn setup_body(code: &str, email: &str) -> Value {
    json!({ "setup_code": code, "email": email, "display_name": "Ana", "password": PASSWORD })
}

async fn post_setup(app: &Router, code: &str, email: &str) -> Res {
    send(
        app,
        req(
            Method::POST,
            "/api/setup",
            Some(setup_body(code, email)),
            &[],
        ),
    )
    .await
}

async fn login(app: &Router, email: &str, password: &str) -> Res {
    login_with(app, email, password, &[]).await
}

async fn login_with(app: &Router, email: &str, password: &str, headers: &[(&str, &str)]) -> Res {
    send(
        app,
        req(
            Method::POST,
            "/api/auth/login",
            Some(json!({ "email": email, "password": password })),
            headers,
        ),
    )
    .await
}

async fn me(app: &Router, headers: &[(&str, &str)]) -> Res {
    send(app, req(Method::GET, "/api/auth/me", None, headers)).await
}

/// A state with the first admin made through the API: (state, app, admin session cookie raw).
async fn with_admin() -> (AppState, Router, String) {
    let state = state_with(&[]).await;
    let code = init_setup_code(&state).await.unwrap().unwrap();
    let app = app_from(&state, 1);
    let res = post_setup(&app, &code, ADMIN).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let raw = res.session(&state).unwrap();
    (state, app, raw)
}

async fn device_token(s: &Store, user_id: &str) -> String {
    let started = s.start_device("Watch", now()).await.unwrap();
    s.approve_device(&started.user_code, user_id, now())
        .await
        .unwrap();
    match s.poll_device(&started.device_code, now()).await.unwrap() {
        PollOutcome::Issued { raw_token } => raw_token,
        _ => panic!("expected a token"),
    }
}

// Setup.

#[tokio::test]
async fn setup_needs_setup_true_then_false() {
    let state = state_with(&[]).await;
    let code = init_setup_code(&state).await.unwrap().unwrap();
    let app = app_from(&state, 1);
    let res = send(&app, req(Method::GET, "/api/setup", None, &[])).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, json!({ "needs_setup": true }));
    assert_eq!(
        post_setup(&app, &code, ADMIN).await.status,
        StatusCode::CREATED
    );
    let res = send(&app, req(Method::GET, "/api/setup", None, &[])).await;
    assert_eq!(res.body, json!({ "needs_setup": false }));
}

#[tokio::test]
async fn setup_with_logged_code_creates_admin_and_signs_in() {
    let state = state_with(&[]).await;
    let code = init_setup_code(&state).await.unwrap().unwrap();
    // XXXX-XXXX from the user-code alphabet.
    assert_eq!(code.len(), 9, "{code}");
    assert_eq!(&code[4..5], "-");
    assert!(
        code.chars()
            .filter(|c| *c != '-')
            .all(|c| "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(c)),
        "{code}"
    );
    // Asking again keeps the same code.
    assert_eq!(
        init_setup_code(&state).await.unwrap().as_deref(),
        Some(&*code)
    );
    let app = app_from(&state, 1);
    // Typed loosely: lowercase, without the dash, with spaces.
    let typed = format!(" {} {} ", &code[..4], &code[5..]).to_lowercase();
    let res = post_setup(&app, &typed, " Admin@X.test ").await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["id"].as_str().unwrap().to_owned();
    assert_eq!(
        res.body,
        json!({ "id": id, "email": ADMIN, "display_name": "Ana", "is_admin": true })
    );
    let raw = res.session(&state).unwrap();
    assert!(!raw.is_empty());
    let res = me(&app, &[("cookie", &cookie(&state, &raw))]).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["id"], json!(id));
    assert_eq!(res.body["is_admin"], json!(true));
}

#[tokio::test]
async fn setup_with_env_token() {
    let state = state_with(&[("SETUP_TOKEN", "envtoken-setup")]).await;
    // With SETUP_TOKEN set, no code is generated (nor logged).
    assert_eq!(init_setup_code(&state).await.unwrap(), None);
    let app = app_from(&state, 1);
    assert_eq!(
        post_setup(&app, "not-it", ADMIN).await.status,
        StatusCode::FORBIDDEN
    );
    let res = post_setup(&app, "envtoken-setup", ADMIN).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["is_admin"], json!(true));
}

#[tokio::test]
async fn setup_wrong_code_is_403() {
    let state = state_with(&[]).await;
    init_setup_code(&state).await.unwrap().unwrap();
    let app = app_from(&state, 1);
    for wrong in ["AAAA-AAAA", "", "x"] {
        let res = post_setup(&app, wrong, ADMIN).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{wrong}");
        assert_eq!(res.body, json!({ "error": "forbidden" }));
    }
    // No code generated at all (no init): every code is wrong.
    let bare = state_with(&[]).await;
    let res = post_setup(&app_from(&bare, 1), "AAAA-AAAA", ADMIN).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(bare.store.user_count().await.unwrap(), 0);
}

#[tokio::test]
async fn second_setup_is_409() {
    let state = state_with(&[("SETUP_TOKEN", "envtoken-setup")]).await;
    let app = app_from(&state, 1);
    assert_eq!(
        post_setup(&app, "envtoken-setup", ADMIN).await.status,
        StatusCode::CREATED
    );
    let res = post_setup(&app, "envtoken-setup", "other@x.test").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "setup_done" }));
    assert_eq!(state.store.user_count().await.unwrap(), 1);
}

#[tokio::test]
async fn setup_code_is_single_use_across_restart_semantics() {
    let state = state_with(&[]).await;
    let code = init_setup_code(&state).await.unwrap().unwrap();
    let app = app_from(&state, 1);
    assert_eq!(
        post_setup(&app, &code, ADMIN).await.status,
        StatusCode::CREATED
    );
    // The in-memory code is gone once used.
    assert_eq!(*state.setup_code.lock().unwrap(), None);
    assert_eq!(
        post_setup(&app, &code, "other@x.test").await.status,
        StatusCode::CONFLICT
    );
    // A "restart" on the same database makes no new code, and the old one still fails.
    let restarted = state_on(state.store.clone(), &[]).await;
    assert_eq!(init_setup_code(&restarted).await.unwrap(), None);
    let res = post_setup(&app_from(&restarted, 1), &code, "other@x.test").await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(state.store.user_count().await.unwrap(), 1);
}

// Sign-in.

#[tokio::test]
async fn login_success_sets_new_session_cookie() {
    let (state, app, setup_raw) = with_admin().await;
    let res = login(&app, "ADMIN@x.test ", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["email"], json!(ADMIN));
    assert_eq!(res.body["is_admin"], json!(true));
    let raw = res.session(&state).unwrap();
    assert!(!raw.is_empty());
    assert_ne!(raw, setup_raw);
    let set = res.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Lax"),
        "{set}"
    );
    assert_eq!(
        me(&app, &[("cookie", &cookie(&state, &raw))]).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn login_replaces_presented_session() {
    let (state, app, old) = with_admin().await;
    let old_cookie = cookie(&state, &old);
    let res = login_with(&app, ADMIN, PASSWORD, &[("cookie", &old_cookie)]).await;
    assert_eq!(res.status, StatusCode::OK);
    let new = res.session(&state).unwrap();
    assert_ne!(new, old);
    // The presented session is gone; the new one works.
    assert_eq!(
        me(&app, &[("cookie", &old_cookie)]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        me(&app, &[("cookie", &cookie(&state, &new))]).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn unknown_email_wrong_password_and_disabled_user_look_identical() {
    let (state, app, _) = with_admin().await;
    let hash = padel_score_server::auth::hash_password(&state.password_permits, PASSWORD)
        .await
        .unwrap();
    let user = state
        .store
        .create_user(
            NewUser {
                email: "off@x.test".into(),
                display_name: "Off".into(),
                password_hash: hash,
            },
            now(),
        )
        .await
        .unwrap();
    state
        .store
        .set_disabled(&user.id, Some(now()))
        .await
        .unwrap();
    let cases = [
        ("nobody@x.test", PASSWORD),
        (ADMIN, "wrong password"),
        ("off@x.test", PASSWORD),
    ];
    let mut seen = Vec::new();
    for (email, pw) in cases {
        let res = login(&app, email, pw).await;
        assert_eq!(res.session(&state), None, "{email}");
        seen.push((res.status, res.body));
    }
    for s in &seen {
        assert_eq!(
            s,
            &(
                StatusCode::UNAUTHORIZED,
                json!({ "error": "invalid_credentials" })
            )
        );
    }
}

#[tokio::test]
async fn login_is_rate_limited_per_email_and_ip() {
    let (state, _, _) = with_admin().await;
    // Per email: 5 failures a minute, from any address.
    for i in 0..5u8 {
        let res = login(&app_from(&state, 10 + i), ADMIN, "wrong password").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
    let res = login(&app_from(&state, 20), ADMIN, PASSWORD).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    let retry: u64 = res.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    // Per address: 10 failures a minute, across emails.
    let app = app_from(&state, 99);
    for i in 0..10 {
        let res = login(&app, &format!("n{i}@x.test"), "wrong password").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
    let res = login(&app, "fresh@x.test", "wrong password").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers.contains_key(header::RETRY_AFTER));
    // Another address is unaffected.
    let res = login(&app_from(&state, 98), "fresh@x.test", "wrong password").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn successful_login_refunds_limits() {
    let (_state, app, _) = with_admin().await;
    for _ in 0..4 {
        assert_eq!(
            login(&app, ADMIN, "wrong password").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    // Successes give their slot back, so they never use up the allowance.
    for _ in 0..3 {
        assert_eq!(login(&app, ADMIN, PASSWORD).await.status, StatusCode::OK);
    }
    assert_eq!(
        login(&app, ADMIN, "wrong password").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, ADMIN, "wrong password").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn concurrent_wrong_logins_bounded_by_limit() {
    let (_state, app, _) = with_admin().await;
    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { login(&app, ADMIN, "wrong password").await.status })
        })
        .collect();
    let mut unauthorized = 0;
    let mut limited = 0;
    for t in tasks {
        match t.await.unwrap() {
            StatusCode::UNAUTHORIZED => unauthorized += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert_eq!(unauthorized, 5);
    assert_eq!(limited, 15);
}

#[tokio::test]
async fn logout_is_idempotent_and_clears_cookie() {
    let (state, app, raw) = with_admin().await;
    let c = cookie(&state, &raw);
    for headers in [
        vec![("cookie", c.as_str())],
        vec![("cookie", c.as_str())],
        vec![],
    ] {
        let res = send(&app, req(Method::POST, "/api/auth/logout", None, &headers)).await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
        assert_eq!(res.session(&state).as_deref(), Some(""));
        let set = res.headers[header::SET_COOKIE].to_str().unwrap();
        assert!(set.contains("Max-Age=0"), "{set}");
    }
    assert_eq!(
        me(&app, &[("cookie", &c)]).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn me_works_with_session_and_device_token() {
    let (state, app, raw) = with_admin().await;
    let res = me(&app, &[("cookie", &cookie(&state, &raw))]).await;
    assert_eq!(res.status, StatusCode::OK);
    let id = res.body["id"].as_str().unwrap().to_owned();
    let token = device_token(&state.store, &id).await;
    let bearer = format!("Bearer {token}");
    let res = me(&app, &[("authorization", &bearer)]).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.body,
        json!({ "id": id, "email": ADMIN, "display_name": "Ana", "is_admin": true })
    );
    let res = me(&app, &[]).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body, json!({ "error": "unauthorized" }));
}

// Password change.

async fn change_password(app: &Router, headers: &[(&str, &str)], current: &str, new: &str) -> Res {
    send(
        app,
        req(
            Method::POST,
            "/api/auth/password",
            Some(json!({ "current_password": current, "new_password": new })),
            headers,
        ),
    )
    .await
}

#[tokio::test]
async fn change_password_requires_current() {
    let (state, app, raw) = with_admin().await;
    let c = cookie(&state, &raw);
    let res = change_password(&app, &[("cookie", &c)], "wrong password", "new password 1").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body, json!({ "error": "invalid_credentials" }));
    // Nothing changed: the old password still signs in, the session still works.
    assert_eq!(me(&app, &[("cookie", &c)]).await.status, StatusCode::OK);
    assert_eq!(login(&app, ADMIN, PASSWORD).await.status, StatusCode::OK);
    // Without a session at all.
    let res = change_password(&app, &[], PASSWORD, "new password 1").await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn change_password_keeps_caller_signed_in_and_revokes_others() {
    let (state, app, caller) = with_admin().await;
    let other = login(&app, ADMIN, PASSWORD).await.session(&state).unwrap();
    let id = state
        .store
        .user_by_email(ADMIN)
        .await
        .unwrap()
        .unwrap()
        .user
        .id;
    let token = device_token(&state.store, &id).await;
    let caller_cookie = cookie(&state, &caller);
    let res = change_password(
        &app,
        &[("cookie", &caller_cookie)],
        PASSWORD,
        "new password 1",
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let fresh = res.session(&state).unwrap();
    assert!(!fresh.is_empty());
    assert_ne!(fresh, caller);
    // The caller stays signed in on the new session; everything else is gone.
    assert_eq!(
        me(&app, &[("cookie", &cookie(&state, &fresh))])
            .await
            .status,
        StatusCode::OK
    );
    for headers in [
        vec![("cookie".to_owned(), caller_cookie.clone())],
        vec![("cookie".to_owned(), cookie(&state, &other))],
        vec![("authorization".to_owned(), format!("Bearer {token}"))],
    ] {
        let h: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(me(&app, &h).await.status, StatusCode::UNAUTHORIZED, "{h:?}");
    }
    assert_eq!(
        login(&app, ADMIN, PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, ADMIN, "new password 1").await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn device_token_cannot_change_password() {
    let (state, app, _) = with_admin().await;
    let id = state
        .store
        .user_by_email(ADMIN)
        .await
        .unwrap()
        .unwrap()
        .user
        .id;
    let bearer = format!("Bearer {}", device_token(&state.store, &id).await);
    let res = change_password(
        &app,
        &[("authorization", &bearer)],
        PASSWORD,
        "new password 1",
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(login(&app, ADMIN, PASSWORD).await.status, StatusCode::OK);
}

// Cross-cutting.

#[tokio::test]
async fn validation_errors_name_the_field() {
    let state = state_with(&[("SETUP_TOKEN", "envtoken-setup")]).await;
    let app = app_from(&state, 1);
    let long_name = "n".repeat(41);
    let long_email = format!("{}@x.test", "e".repeat(250));
    let long_pw = "p".repeat(129);
    let cases = [
        (
            json!({ "email": "no-at-sign", "display_name": "Ana", "password": PASSWORD }),
            "email",
        ),
        (
            json!({ "email": "@", "display_name": "Ana", "password": PASSWORD }),
            "email",
        ),
        (
            json!({ "email": long_email, "display_name": "Ana", "password": PASSWORD }),
            "email",
        ),
        (
            json!({ "email": ADMIN, "display_name": "   ", "password": PASSWORD }),
            "display_name",
        ),
        (
            json!({ "email": ADMIN, "display_name": long_name, "password": PASSWORD }),
            "display_name",
        ),
        (
            json!({ "email": ADMIN, "display_name": "Ana", "password": "short" }),
            "password",
        ),
        (
            json!({ "email": ADMIN, "display_name": "Ana", "password": long_pw }),
            "password",
        ),
    ];
    for (mut body, field) in cases {
        body["setup_code"] = json!("envtoken-setup");
        let res = send(
            &app,
            req(Method::POST, "/api/setup", Some(body.clone()), &[]),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(
            res.body,
            json!({ "error": "validation", "field": field }),
            "{body}"
        );
    }
    assert_eq!(state.store.user_count().await.unwrap(), 0);
    // A 40-char trimmed name is fine.
    let mut ok = setup_body("envtoken-setup", ADMIN);
    ok["display_name"] = json!(format!("  {}  ", "n".repeat(40)));
    let res = send(&app, req(Method::POST, "/api/setup", Some(ok), &[])).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["display_name"], json!("n".repeat(40)));
    // The new password on a change is held to the policy too.
    let raw = res.session(&state).unwrap();
    let res = change_password(
        &app,
        &[("cookie", &cookie(&state, &raw))],
        PASSWORD,
        "short",
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        res.body,
        json!({ "error": "validation", "field": "new_password" })
    );
}

#[tokio::test]
async fn auth_responses_have_no_referrer_policy() {
    let (state, app, raw) = with_admin().await;
    let c = cookie(&state, &raw);
    let responses = [
        send(&app, req(Method::GET, "/api/setup", None, &[])).await,
        post_setup(&app, "AAAA-AAAA", "x@x.test").await,
        login(&app, ADMIN, "wrong password").await,
        login(&app, ADMIN, PASSWORD).await,
        me(&app, &[("cookie", &c)]).await,
        me(&app, &[]).await,
        change_password(&app, &[("cookie", &c)], "wrong password", "new password 1").await,
        send(&app, req(Method::POST, "/api/auth/logout", None, &[])).await,
    ];
    for res in responses {
        assert_eq!(
            res.headers
                .get(header::REFERRER_POLICY)
                .map(|v| v.to_str().unwrap()),
            Some("no-referrer"),
            "{} {}",
            res.status,
            res.body
        );
    }
}

#[tokio::test]
async fn anonymous_quick_match_still_works_after_setup() {
    let (_state, app, _) = with_admin().await;
    let body = json!({
        "format": Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::One,
            deciding: DecidingSet::FullSet,
        }),
        "start": Start {
            first_server: Player { team: Team::A, index: 0 },
            other_first_server: 0,
        },
        "teams": { "a": ["Ana", "Bea"], "b": ["Cris", "Dani"] },
    });
    let create = Request::post("/api/matches")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = send(&app, create).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let scorer = res.body["scorer_token"].as_str().unwrap().to_owned();
    let point = json!({
        "client_id": "c1", "batch": false, "base_revision": 0,
        "commands": [{ "kind": "point", "team": "a" }],
    });
    let cmd = Request::post(format!("/api/score/{scorer}/commands"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(point.to_string()))
        .unwrap();
    assert_eq!(send(&app, cmd).await.status, StatusCode::OK);
    let res = send(
        &app,
        Request::get(format!("/api/score/{scorer}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.body["revision"], json!(1));
    // Phase 2 responses are untouched by the auth headers.
    assert!(!res.headers.contains_key(header::REFERRER_POLICY));
}

#[tokio::test]
async fn change_password_wrong_current_is_rate_limited() {
    let (state, app, raw) = with_admin().await;
    let c = cookie(&state, &raw);
    for i in 0..5 {
        let res =
            change_password(&app, &[("cookie", &c)], "wrong password", "new password 1").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
    let res = change_password(&app, &[("cookie", &c)], "wrong password", "new password 1").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers.contains_key(header::RETRY_AFTER));
    // Refused before the check: even the right password is held off now, and nothing changed.
    let res = change_password(&app, &[("cookie", &c)], PASSWORD, "new password 1").await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(me(&app, &[("cookie", &c)]).await.status, StatusCode::OK);
}

#[tokio::test]
async fn successful_password_change_refunds_limits() {
    let (state, app, raw) = with_admin().await;
    let mut c = cookie(&state, &raw);
    for _ in 0..4 {
        change_password(&app, &[("cookie", &c)], "wrong password", "new password 1").await;
    }
    // Two successes in a row (swapping passwords) both pass: each refunds its slot.
    for (current, new) in [(PASSWORD, "new password 1"), ("new password 1", PASSWORD)] {
        let res = change_password(&app, &[("cookie", &c)], current, new).await;
        assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
        c = cookie(&state, &res.session(&state).unwrap());
    }
}

#[tokio::test]
async fn ip_refusal_does_not_create_email_windows() {
    let (state, _, _) = with_admin().await;
    let app = app_from(&state, 77);
    for i in 0..10 {
        let res = login(&app, &format!("n{i}@x.test"), "wrong password").await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
    let before = state.limits.login_email.len();
    assert_eq!(before, 10);
    for i in 0..5 {
        let res = login(&app, &format!("fresh{i}@x.test"), "wrong password").await;
        assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    }
    assert_eq!(state.limits.login_email.len(), before);
}
