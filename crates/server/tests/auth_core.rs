#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The auth core behind small test-only routes: the extractors, the CSRF guard and the new
//! `ApiError` variants. Credentials are minted through the store at the real clock, because the
//! extractors read it.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use http_body_util::BodyExt;
use padel_score_server::accounts_store::{NewUser, PollOutcome};
use padel_score_server::api::{ApiError, now};
use padel_score_server::auth::{AdminUser, AuthVia, CurrentUser, OptionalUser, SessionUser};
use padel_score_server::{AppState, Config, Store, db, router};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "padel.local:8080";

async fn state_with(env: &[(&str, &str)]) -> AppState {
    let map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let store = Store::new(db::connect("sqlite::memory:").await.unwrap());
    AppState::new(Arc::new(Config::from_map(&map).unwrap()), store)
}

fn via_json(via: &AuthVia) -> Value {
    match via {
        AuthVia::Session => json!("session"),
        AuthVia::Device { token_id } => json!({ "device": token_id }),
    }
}

async fn whoami(user: CurrentUser) -> Json<Value> {
    Json(json!({ "id": user.user.id, "via": via_json(&user.via) }))
}

async fn maybe(OptionalUser(user): OptionalUser) -> Json<Value> {
    Json(json!({ "id": user.map(|u| u.user.id) }))
}

async fn session_only(SessionUser(user): SessionUser) -> Json<Value> {
    Json(json!({ "id": user.user.id }))
}

async fn admin_only(AdminUser(user): AdminUser) -> Json<Value> {
    Json(json!({ "id": user.user.id }))
}

fn test_routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/t/me",
            get(whoami)
                .post(whoami)
                .put(whoami)
                .patch(whoami)
                .delete(whoami),
        )
        .route("/t/maybe", get(maybe).post(maybe))
        .route("/t/session", get(session_only).post(session_only))
        .route("/t/admin", get(admin_only))
        .with_state(state)
}

fn new_user(email: &str) -> NewUser {
    NewUser {
        email: email.into(),
        display_name: format!("User {email}"),
        password_hash: "$argon2id$fake".into(),
    }
}

/// An admin plus a regular user: their ids.
async fn admin_and_user(s: &Store) -> (String, String) {
    let admin = s
        .create_first_admin(new_user("admin@x.test"), now())
        .await
        .unwrap();
    let user = s.create_user(new_user("user@x.test"), now()).await.unwrap();
    (admin.id, user.id)
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

fn cookie(state: &AppState, raw: &str) -> String {
    format!(
        "{}={raw}",
        padel_score_server::auth::cookie_name(&state.config)
    )
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
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
    (status, headers, body)
}

/// A request to `path` with the given headers; the Host is always [`HOST`].
fn req(method: Method, path: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Body::empty()).unwrap()
}

async fn status_of(
    app: &Router,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
) -> StatusCode {
    send(app, req(method, path, headers)).await.0
}

#[tokio::test]
async fn bearer_device_token_is_accepted() {
    let state = state_with(&[]).await;
    let (_, user) = admin_and_user(&state.store).await;
    let token = device_token(&state.store, &user).await;
    let device_id = state.store.list_devices(&user).await.unwrap()[0].id.clone();
    let app = test_routes(state);
    let bearer = format!("Bearer {token}");
    let (status, _, body) = send(
        &app,
        req(Method::GET, "/t/me", &[("authorization", &bearer)]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "id": user, "via": { "device": device_id } }));
    // A bad pdl_ bearer is a 401 even next to a good cookie.
    let (status, _, body) = send(
        &app,
        req(
            Method::GET,
            "/t/me",
            &[("authorization", "Bearer pdl_nope")],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, json!({ "error": "unauthorized" }));
}

#[tokio::test]
async fn session_cookie_is_accepted() {
    let state = state_with(&[]).await;
    let (_, user) = admin_and_user(&state.store).await;
    let raw = state.store.create_session(&user, now()).await.unwrap();
    let c = cookie(&state, &raw);
    let app = test_routes(state.clone());
    let with_others = format!("theme=dark; {c}; other=1");
    let (status, _, body) =
        send(&app, req(Method::GET, "/t/me", &[("cookie", &with_others)])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "id": user, "via": "session" }));
    // A non-pdl Authorization is not ours: the cookie still decides.
    let (status, _, _) = send(
        &app,
        req(
            Method::GET,
            "/t/me",
            &[("cookie", &c), ("authorization", "Basic Zm9vOmJhcg==")],
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // No credential at all.
    let (status, _, body) = send(&app, req(Method::GET, "/t/me", &[])).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, json!({ "error": "unauthorized" }));
    // The cookie under the other setting's name is not read.
    let wrong_name = format!("padel_session={raw}");
    assert_eq!(
        status_of(&app, Method::GET, "/t/me", &[("cookie", &wrong_name)]).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn disabled_user_gets_401() {
    let state = state_with(&[]).await;
    let (_, user) = admin_and_user(&state.store).await;
    let raw = state.store.create_session(&user, now()).await.unwrap();
    let token = device_token(&state.store, &user).await;
    let c = cookie(&state, &raw);
    let bearer = format!("Bearer {token}");
    let app = test_routes(state.clone());
    assert_eq!(
        status_of(&app, Method::GET, "/t/me", &[("cookie", &c)]).await,
        StatusCode::OK
    );
    state.store.set_disabled(&user, Some(now())).await.unwrap();
    assert_eq!(
        status_of(&app, Method::GET, "/t/me", &[("cookie", &c)]).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(&app, Method::GET, "/t/me", &[("authorization", &bearer)]).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn bad_cookie_is_anonymous_for_optional_user() {
    let state = state_with(&[]).await;
    let (_, user) = admin_and_user(&state.store).await;
    let raw = state.store.create_session(&user, now()).await.unwrap();
    let app = test_routes(state.clone());
    let bad = cookie(&state, "not-a-session");
    for headers in [
        vec![("cookie", bad.as_str())],
        vec![("authorization", "Bearer pdl_nope")],
        vec![],
    ] {
        let (status, _, body) = send(&app, req(Method::GET, "/t/maybe", &headers)).await;
        assert_eq!(status, StatusCode::OK, "{headers:?}");
        assert_eq!(body, json!({ "id": null }), "{headers:?}");
        // An unsafe method too: no CSRF refusal for an anonymous caller.
        let (status, _, body) = send(&app, req(Method::POST, "/t/maybe", &headers)).await;
        assert_eq!(status, StatusCode::OK, "{headers:?}");
        assert_eq!(body, json!({ "id": null }), "{headers:?}");
    }
    let good = cookie(&state, &raw);
    let (_, _, body) = send(&app, req(Method::GET, "/t/maybe", &[("cookie", &good)])).await;
    assert_eq!(body, json!({ "id": user }));
    // A revoked session reads as anonymous too.
    state.store.delete_session(&raw).await.unwrap();
    let (status, _, body) = send(&app, req(Method::GET, "/t/maybe", &[("cookie", &good)])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "id": null }));
}

#[tokio::test]
async fn device_token_on_session_only_route_is_403() {
    let state = state_with(&[]).await;
    let (admin, user) = admin_and_user(&state.store).await;
    let token = device_token(&state.store, &user).await;
    let admin_token = device_token(&state.store, &admin).await;
    let raw = state.store.create_session(&user, now()).await.unwrap();
    let c = cookie(&state, &raw);
    let app = test_routes(state);
    let bearer = format!("Bearer {token}");
    let (status, _, body) = send(
        &app,
        req(Method::GET, "/t/session", &[("authorization", &bearer)]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({ "error": "forbidden" }));
    // An admin's device token cannot reach admin routes either.
    let admin_bearer = format!("Bearer {admin_token}");
    assert_eq!(
        status_of(
            &app,
            Method::GET,
            "/t/admin",
            &[("authorization", &admin_bearer)]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let (status, _, body) = send(&app, req(Method::GET, "/t/session", &[("cookie", &c)])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "id": user }));
    assert_eq!(
        status_of(&app, Method::GET, "/t/session", &[]).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn non_admin_on_admin_route_is_403() {
    let state = state_with(&[]).await;
    let (admin, user) = admin_and_user(&state.store).await;
    let user_c = cookie(
        &state,
        &state.store.create_session(&user, now()).await.unwrap(),
    );
    let admin_c = cookie(
        &state,
        &state.store.create_session(&admin, now()).await.unwrap(),
    );
    let app = test_routes(state);
    let (status, _, body) = send(&app, req(Method::GET, "/t/admin", &[("cookie", &user_c)])).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, json!({ "error": "forbidden" }));
    let (status, _, body) = send(&app, req(Method::GET, "/t/admin", &[("cookie", &admin_c)])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "id": admin }));
    assert_eq!(
        status_of(&app, Method::GET, "/t/admin", &[]).await,
        StatusCode::UNAUTHORIZED
    );
}

/// A signed-in user's cookie header and the test router.
async fn signed_in(env: &[(&str, &str)]) -> (Router, String, AppState) {
    let state = state_with(env).await;
    let (_, user) = admin_and_user(&state.store).await;
    let raw = state.store.create_session(&user, now()).await.unwrap();
    let c = cookie(&state, &raw);
    (test_routes(state.clone()), c, state)
}

#[tokio::test]
async fn csrf_cross_site_post_with_cookie_is_403() {
    let (app, c, _) = signed_in(&[]).await;
    for extra in [
        vec![("sec-fetch-site", "cross-site")],
        vec![("sec-fetch-site", "same-site")],
        // Sec-Fetch-Site decides when present, even next to a matching Origin.
        vec![
            ("sec-fetch-site", "cross-site"),
            ("origin", "http://padel.local:8080"),
        ],
        vec![("origin", "https://evil.test")],
        vec![("origin", "null")],
        // Neither header: refused.
        vec![],
    ] {
        let mut headers = vec![("cookie", c.as_str())];
        headers.extend(extra.iter().copied());
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let (status, _, body) = send(&app, req(method.clone(), "/t/me", &headers)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {extra:?}");
            assert_eq!(body, json!({ "error": "forbidden" }));
        }
        assert_eq!(
            status_of(&app, Method::POST, "/t/session", &headers).await,
            StatusCode::FORBIDDEN,
            "{extra:?}"
        );
        // Safe methods are not guarded.
        assert_eq!(
            status_of(&app, Method::GET, "/t/me", &headers).await,
            StatusCode::OK,
            "{extra:?}"
        );
        // An optional route acts anonymously rather than as the user.
        let (status, _, body) = send(&app, req(Method::POST, "/t/maybe", &headers)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "id": null }), "{extra:?}");
    }
}

#[tokio::test]
async fn csrf_same_origin_post_is_ok() {
    let (app, c, _) = signed_in(&[]).await;
    for site in ["same-origin", "none", "Same-Origin"] {
        let headers = [("cookie", c.as_str()), ("sec-fetch-site", site)];
        assert_eq!(
            status_of(&app, Method::POST, "/t/me", &headers).await,
            StatusCode::OK,
            "{site}"
        );
        let (_, _, body) = send(&app, req(Method::POST, "/t/maybe", &headers)).await;
        assert!(body["id"].is_string(), "{site}");
    }
}

#[tokio::test]
async fn csrf_origin_must_match_public_origin() {
    let (app, c, _) = signed_in(&[("PUBLIC_ORIGIN", "https://padel.example.test/")]).await;
    let post = |origin: &'static str| {
        let app = app.clone();
        let c = c.clone();
        async move {
            status_of(
                &app,
                Method::POST,
                "/t/me",
                &[("cookie", &c), ("origin", origin)],
            )
            .await
        }
    };
    assert_eq!(post("https://padel.example.test").await, StatusCode::OK);
    assert_eq!(post("https://PADEL.example.test").await, StatusCode::OK);
    assert_eq!(
        post("http://padel.example.test").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(post("https://evil.test").await, StatusCode::FORBIDDEN);
    // With PUBLIC_ORIGIN set, matching the Host header is not enough.
    assert_eq!(post("http://padel.local:8080").await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_unset_public_origin_compares_host() {
    let (app, c, _) = signed_in(&[]).await;
    let post = |origin: &'static str| {
        let app = app.clone();
        let c = c.clone();
        async move {
            status_of(
                &app,
                Method::POST,
                "/t/me",
                &[("cookie", &c), ("origin", origin)],
            )
            .await
        }
    };
    assert_eq!(post("http://padel.local:8080").await, StatusCode::OK);
    assert_eq!(post("https://padel.local:8080").await, StatusCode::OK);
    assert_eq!(post("http://padel.local").await, StatusCode::FORBIDDEN);
    assert_eq!(post("http://padel.local:8081").await, StatusCode::FORBIDDEN);
    assert_eq!(post("http://evil.test").await, StatusCode::FORBIDDEN);
    assert_eq!(
        post("http://padel.local:8080.evil.test").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(post("padel.local:8080").await, StatusCode::FORBIDDEN);
    // No Host header to compare against: refused.
    let req = Request::post("/t/me")
        .header("cookie", &c)
        .header("origin", "http://padel.local:8080")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn bearer_post_skips_csrf() {
    let state = state_with(&[("PUBLIC_ORIGIN", "https://padel.example.test")]).await;
    let (_, user) = admin_and_user(&state.store).await;
    let token = device_token(&state.store, &user).await;
    let app = test_routes(state);
    let bearer = format!("Bearer {token}");
    for extra in [
        vec![],
        vec![("sec-fetch-site", "cross-site")],
        vec![("origin", "https://evil.test")],
    ] {
        let mut headers = vec![("authorization", bearer.as_str())];
        headers.extend(extra.iter().copied());
        assert_eq!(
            status_of(&app, Method::POST, "/t/me", &headers).await,
            StatusCode::OK,
            "{extra:?}"
        );
        let (_, _, body) = send(&app, req(Method::POST, "/t/maybe", &headers)).await;
        assert_eq!(body, json!({ "id": user }), "{extra:?}");
    }
}

#[tokio::test]
async fn anonymous_quick_match_still_works() {
    let state = state_with(&[]).await;
    let app = router(state);
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
    // A cross-site POST with no cookie, as a plain anonymous browser sends it.
    let create = Request::post("/api/matches")
        .header(header::CONTENT_TYPE, "application/json")
        .header("sec-fetch-site", "cross-site")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, _, created) = send(&app, create).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let scorer = created["scorer_token"].as_str().unwrap();
    let point = json!({
        "client_id": "c1", "batch": false, "base_revision": 0,
        "commands": [{ "kind": "point", "team": "a" }],
    });
    let cmd = Request::post(format!("/api/score/{scorer}/commands"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(point.to_string()))
        .unwrap();
    let (status, _, applied) = send(&app, cmd).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let (status, _, view) = send(
        &app,
        Request::get(format!("/api/score/{scorer}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["revision"], json!(1), "{view}");
}

#[tokio::test]
async fn too_many_has_retry_after_header() {
    let res = ApiError::TooMany { retry_after: 42 }.into_response();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.headers()[header::RETRY_AFTER], "42");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        json!({ "error": "too_many_requests", "retry_after_secs": 42 })
    );

    let res = ApiError::Validation { field: "password" }.into_response();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body, json!({ "error": "validation", "field": "password" }));
}
