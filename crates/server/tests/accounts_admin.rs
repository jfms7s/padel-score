#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The admin routes (list, disable, enable, reset links) and the public reset, through the real
//! router; plus the request logs, which must never carry an invite or reset code.

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{Method, StatusCode};
use padel_score_server::accounts_store::RESET_TTL_SECS;
use padel_score_server::api::now;
use padel_score_server::store::Teams;
use padel_score_server::{AppState, router, trace_layer};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use support::*;

const NEW_PASSWORD: &str = "a brand new password";

async fn admin_post(state: &AppState, app: &Router, admin: &str, path: &str) -> Res {
    send(app, as_session(state, admin, Method::POST, path, None)).await
}

async fn disable(state: &AppState, app: &Router, admin: &str, id: &str) -> Res {
    admin_post(state, app, admin, &format!("/api/admin/users/{id}/disable")).await
}

async fn enable(state: &AppState, app: &Router, admin: &str, id: &str) -> Res {
    admin_post(state, app, admin, &format!("/api/admin/users/{id}/enable")).await
}

/// A reset link for `id`: its code.
async fn reset_link(state: &AppState, app: &Router, admin: &str, id: &str) -> String {
    let path = format!("/api/admin/users/{id}/reset-link");
    let res = admin_post(state, app, admin, &path).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    fragment_code(res.body["url"].as_str().unwrap())
}

async fn reset(app: &Router, code: &str, password: &str) -> Res {
    let body = json!({ "code": code, "password": password });
    send(app, req(Method::POST, "/api/reset", Some(body), &[])).await
}

async fn me_by_device(app: &Router, token: &str) -> StatusCode {
    send(app, as_device(token, Method::GET, "/api/auth/me", None))
        .await
        .status
}

/// Make `id` an admin directly in the database (there is no promote route yet).
async fn promote(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query("UPDATE users SET is_admin = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn admin_lists_users_with_inviter() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let (cy_id, _) = invited_user(&state, &app, &bea, "cy@x.test", "Cy").await;
    assert_eq!(
        disable(&state, &app, &admin, &cy_id).await.status,
        StatusCode::NO_CONTENT
    );
    let res = send(
        &app,
        as_session(&state, &admin, Method::GET, "/api/admin/users", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let users = res.body.as_array().unwrap();
    assert_eq!(users.len(), 3);
    let by_id = |id: &str| -> &Value { users.iter().find(|u| u["id"] == json!(id)).unwrap() };
    let created = |id: &str| by_id(id)["created_at"].as_i64().unwrap();
    assert_eq!(
        by_id(&admin_id),
        &json!({
            "id": admin_id, "email": ADMIN, "display_name": "Ana", "is_admin": true,
            "disabled": false, "created_at": created(&admin_id),
        })
    );
    assert_eq!(
        by_id(&bea_id),
        &json!({
            "id": bea_id, "email": "bea@x.test", "display_name": "Bea", "is_admin": false,
            "disabled": false, "invited_by_name": "Ana", "created_at": created(&bea_id),
        })
    );
    let cy = by_id(&cy_id);
    assert_eq!(cy["invited_by_name"], json!("Bea"));
    assert_eq!(cy["disabled"], json!(true));
    // No secret of any kind.
    let text = res.body.to_string();
    assert!(
        !text.contains("password") && !text.contains("argon2"),
        "{text}"
    );
}

#[tokio::test]
async fn non_admin_gets_403() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let (_, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let paths = [
        (Method::GET, "/api/admin/users".to_owned()),
        (Method::POST, format!("/api/admin/users/{admin_id}/disable")),
        (Method::POST, format!("/api/admin/users/{admin_id}/enable")),
        (
            Method::POST,
            format!("/api/admin/users/{admin_id}/reset-link"),
        ),
    ];
    for (method, path) in &paths {
        let res = send(&app, as_session(&state, &bea, method.clone(), path, None)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(res.body, json!({ "error": "forbidden" }));
        let res = send(&app, req(method.clone(), path, None, &[])).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    // The admin is untouched.
    assert_eq!(me(&state, &app, &admin).await.status, StatusCode::OK);
    // An unknown user id is 404 for the admin.
    for suffix in ["disable", "enable", "reset-link"] {
        let res = admin_post(
            &state,
            &app,
            &admin,
            &format!("/api/admin/users/nope/{suffix}"),
        )
        .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{suffix}");
        assert_eq!(res.body, json!({ "error": "not_found" }));
    }
}

#[tokio::test]
async fn disable_signs_user_out_everywhere() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let token = device_token(&state.store, &bea_id).await;
    assert_eq!(me(&state, &app, &bea).await.status, StatusCode::OK);
    assert_eq!(me_by_device(&app, &token).await, StatusCode::OK);
    let res = disable(&state, &app, &admin, &bea_id).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    assert_eq!(res.body, Value::Null);
    assert_eq!(
        me(&state, &app, &bea).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(me_by_device(&app, &token).await, StatusCode::UNAUTHORIZED);
    // Signing in again is refused like a wrong password.
    let res = login(&app, "bea@x.test", PASSWORD).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body, json!({ "error": "invalid_credentials" }));
    // Disabling twice is fine.
    assert_eq!(
        disable(&state, &app, &admin, &bea_id).await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn cannot_disable_self() {
    let (state, pool, app, admin_id, admin) = with_admin().await;
    let res = disable(&state, &app, &admin, &admin_id).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "cannot_disable_self" }));
    // Even with another admin around.
    let (bea_id, _) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    promote(&pool, &bea_id).await;
    let res = disable(&state, &app, &admin, &admin_id).await;
    assert_eq!(res.body, json!({ "error": "cannot_disable_self" }));
    assert_eq!(me(&state, &app, &admin).await.status, StatusCode::OK);
}

#[tokio::test]
async fn last_admin_protected() {
    let (state, pool, app, ana_id, ana) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &ana, "bea@x.test", "Bea").await;
    promote(&pool, &bea_id).await;
    // Two admins disable each other at once: both pass the admin check, then the store
    // serializes the two disables, and the second would leave no enabled admin.
    let (a, b) = tokio::join!(
        disable(&state, &app, &ana, &bea_id),
        disable(&state, &app, &bea, &ana_id),
    );
    let mut outcomes = [(a.status, a.body), (b.status, b.body)];
    outcomes.sort_by_key(|(s, _)| *s);
    assert_eq!(outcomes[0], (StatusCode::NO_CONTENT, Value::Null));
    assert_eq!(
        outcomes[1],
        (StatusCode::CONFLICT, json!({ "error": "last_admin" }))
    );
    let enabled_admins = [&ana_id, &bea_id].len()
        - [
            state.store.user_by_id(&ana_id).await.unwrap().unwrap(),
            state.store.user_by_id(&bea_id).await.unwrap().unwrap(),
        ]
        .iter()
        .filter(|u| u.disabled_at.is_some())
        .count();
    assert_eq!(enabled_admins, 1);
}

#[tokio::test]
async fn enable_restores_login() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, _) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    disable(&state, &app, &admin, &bea_id).await;
    assert_eq!(
        login(&app, "bea@x.test", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = enable(&state, &app, &admin, &bea_id).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = login(&app, "bea@x.test", PASSWORD).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let raw = res.session(&state).unwrap();
    assert_eq!(me(&state, &app, &raw).await.status, StatusCode::OK);
    // Enabling an enabled user is fine.
    assert_eq!(
        enable(&state, &app, &admin, &bea_id).await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn reset_link_single_use_and_revokes() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let token = device_token(&state.store, &bea_id).await;
    let before = now();
    let path = format!("/api/admin/users/{bea_id}/reset-link");
    let res = admin_post(&state, &app, &admin, &path).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let mut keys: Vec<&str> = res
        .body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["expires_at", "url"]);
    let url = res.body["url"].as_str().unwrap();
    let (prefix, code) = url.split_once('#').unwrap();
    assert_eq!(prefix, "/reset");
    assert!(code.len() >= 40);
    let expires_at = res.body["expires_at"].as_i64().unwrap();
    assert!((before + RESET_TTL_SECS..=now() + RESET_TTL_SECS).contains(&expires_at));
    // The password policy applies.
    let res = reset(&app, code, "short").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        res.body,
        json!({ "error": "validation", "field": "password" })
    );
    let res = reset(&app, code, NEW_PASSWORD).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    // Every session and device token of the user ends.
    assert_eq!(
        me(&state, &app, &bea).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(me_by_device(&app, &token).await, StatusCode::UNAUTHORIZED);
    // Single use.
    let res = reset(&app, code, "yet another password").await;
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body, json!({ "error": "reset_invalid" }));
    let res = reset(&app, "not-a-code", NEW_PASSWORD).await;
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body, json!({ "error": "reset_invalid" }));
    // The new password works, the old one does not.
    assert_eq!(
        login(&app, "bea@x.test", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(&app, "bea@x.test", NEW_PASSWORD).await.status,
        StatusCode::OK
    );
    // A newer link replaces nothing it should not: issuing two, using one, kills the other.
    let first = reset_link(&state, &app, &admin, &bea_id).await;
    let second = reset_link(&state, &app, &admin, &bea_id).await;
    assert_eq!(
        reset(&app, &second, PASSWORD).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        reset(&app, &first, NEW_PASSWORD).await.status,
        StatusCode::GONE
    );
}

#[tokio::test]
async fn reset_link_expires_after_24h() {
    let (state, pool, app, _, admin) = with_admin().await;
    let (bea_id, _) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let fresh = reset_link(&state, &app, &admin, &bea_id).await;
    let stale = reset_link(&state, &app, &admin, &bea_id).await;
    // Age both links: one to a second short of 24 hours, one to 24 hours.
    let age = |code: &str, secs: i64| {
        let pool = pool.clone();
        let hash = padel_score_server::tokens::hash_token(code);
        async move {
            sqlx::query(
                "UPDATE password_resets SET created_at = created_at - ?, \
                 expires_at = expires_at - ? WHERE code_hash = ?",
            )
            .bind(secs)
            .bind(secs)
            .bind(hash)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    age(&stale, RESET_TTL_SECS).await;
    age(&fresh, RESET_TTL_SECS - 60).await;
    let res = reset(&app, &stale, NEW_PASSWORD).await;
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body, json!({ "error": "reset_invalid" }));
    assert_eq!(
        reset(&app, &fresh, NEW_PASSWORD).await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn reset_does_not_sign_in() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, _) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let code = reset_link(&state, &app, &admin, &bea_id).await;
    let res = reset(&app, &code, NEW_PASSWORD).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(res.session(&state).is_none(), "{:?}", res.headers);
    assert_eq!(res.body, Value::Null);
    // The admin who made the link is not signed out.
    assert_eq!(me(&state, &app, &admin).await.status, StatusCode::OK);
}

#[tokio::test]
async fn reset_is_rate_limited() {
    let (state, _, app, _, _) = with_admin().await;
    for _ in 0..120 {
        assert_eq!(
            reset(&app, "guess", NEW_PASSWORD).await.status,
            StatusCode::GONE
        );
    }
    let res = reset(&app, "guess", NEW_PASSWORD).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    let other = app_from(&state, 2);
    assert_eq!(
        reset(&other, "guess", NEW_PASSWORD).await.status,
        StatusCode::GONE
    );
}

#[tokio::test]
async fn disable_revokes_outstanding_invites() {
    let (state, pool, app, _, admin) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let (_, code) = create_invite(&state, &app, &bea).await;
    let (_, admins) = create_invite(&state, &app, &admin).await;
    assert_eq!(
        disable(&state, &app, &admin, &bea_id).await.status,
        StatusCode::NO_CONTENT
    );
    let revoked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM invites WHERE created_by = ? AND revoked_at IS NOT NULL",
    )
    .bind(&bea_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(revoked, 1);
    // Enabling her again does not bring the invite back.
    assert_eq!(
        enable(&state, &app, &admin, &bea_id).await.status,
        StatusCode::NO_CONTENT
    );
    let res = accept(&app, &code, accept_body("cy@x.test", "Cy")).await;
    assert_eq!(res.status, StatusCode::GONE, "{}", res.body);
    // Other users' invites are untouched.
    let res = accept(&app, &admins, accept_body("dee@x.test", "Dee")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

#[tokio::test]
async fn reset_with_invalid_code_skips_the_hash() {
    let (state, _, app, _, _) = with_admin().await;
    // Hold every hash permit: a reset that hashed would wait for one.
    let permits = state.password_permits.clone();
    let _held = permits
        .acquire_many_owned(padel_score_server::auth::HASH_PERMITS as u32)
        .await
        .unwrap();
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reset(&app, "not-a-reset-code", NEW_PASSWORD),
    )
    .await
    .expect("an invalid code waited for a hash permit");
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body, json!({ "error": "reset_invalid" }));
    // The policy still comes first.
    let res = reset(&app, "not-a-reset-code", "short").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn reset_with_used_code_skips_the_hash() {
    let (state, _, app, _, admin) = with_admin().await;
    let (bea_id, _) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let code = reset_link(&state, &app, &admin, &bea_id).await;
    assert_eq!(
        reset(&app, &code, NEW_PASSWORD).await.status,
        StatusCode::NO_CONTENT
    );
    let permits = state.password_permits.clone();
    let _held = permits
        .acquire_many_owned(padel_score_server::auth::HASH_PERMITS as u32)
        .await
        .unwrap();
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reset(&app, &code, NEW_PASSWORD),
    )
    .await
    .expect("a used code waited for a hash permit");
    assert_eq!(res.status, StatusCode::GONE);
}

#[tokio::test]
async fn device_token_cannot_use_admin_routes() {
    let (state, _, app, admin_id, _) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let paths = [
        (Method::GET, "/api/admin/users".to_owned()),
        (Method::POST, format!("/api/admin/users/{admin_id}/disable")),
        (Method::POST, format!("/api/admin/users/{admin_id}/enable")),
        (
            Method::POST,
            format!("/api/admin/users/{admin_id}/reset-link"),
        ),
    ];
    for (method, path) in &paths {
        let res = send(&app, as_device(&token, method.clone(), path, None)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(res.body, json!({ "error": "forbidden" }));
    }
    assert_eq!(me_by_device(&app, &token).await, StatusCode::OK);
}

// Logging.

/// A `tracing` writer that keeps everything written to it.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[tokio::test]
async fn codes_never_appear_in_logs() {
    let (state, _, _, _, admin) = with_admin().await;
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    // The router as main.rs serves it: with the request trace layer.
    let app = router(state.clone())
        .layer(trace_layer())
        .layer(MockConnectInfo(peer(1)));

    let (_, code) = create_invite(&state, &app, &admin).await;
    let lookup = json!({ "code": code });
    assert_eq!(
        send(
            &app,
            req(Method::POST, "/api/invites/lookup", Some(lookup), &[])
        )
        .await
        .status,
        StatusCode::OK
    );
    let res = accept(&app, &code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::CREATED);
    let bea_id = res.body["id"].as_str().unwrap().to_owned();
    let reset_code = reset_link(&state, &app, &admin, &bea_id).await;
    assert_eq!(
        reset(&app, &reset_code, NEW_PASSWORD).await.status,
        StatusCode::NO_CONTENT
    );
    // Failures too, including a method no route takes and a code put in a path by mistake.
    let res = send(
        &app,
        req(Method::PUT, &format!("/api/invites/{code}"), None, &[]),
    )
    .await;
    assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
    let res = send(
        &app,
        req(
            Method::GET,
            &format!("/api/invites/{code}/x?c={code}"),
            None,
            &[],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(
        accept(&app, &code, accept_body("cy@x.test", "Cy"))
            .await
            .status,
        StatusCode::GONE
    );
    assert_eq!(
        reset(&app, &reset_code, NEW_PASSWORD).await.status,
        StatusCode::GONE
    );

    // The PWA's scorer page carries a scorer token in its path; no route matches it.
    let m = state
        .store
        .create_match(
            padel_score_server::store::NewMatch {
                format: Format::Sets(SetRules {
                    deuce: DeuceRule::Advantage,
                    sets: SetsFormat::One,
                    deciding: DecidingSet::FullSet,
                }),
                start: Start {
                    first_server: Player {
                        team: Team::A,
                        index: 0,
                    },
                    other_first_server: 0,
                },
                teams: Teams {
                    a: ["A".to_owned(), "B".to_owned()],
                    b: ["C".to_owned(), "D".to_owned()],
                },
                owner_user_id: None,
            },
            now(),
        )
        .await
        .unwrap();
    let page = format!("/s/{}", m.scorer_token);
    let _ = tower::ServiceExt::oneshot(
        app.clone(),
        axum::http::Request::get(&page)
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    let text = logs.text();
    assert!(!text.contains(&code), "invite code logged:\n{text}");
    // The capture works: the trace layer logged these requests, by route.
    assert!(text.contains("/api/invites/lookup"), "{text}");
    assert!(text.contains("/api/invites/accept"), "{text}");
    assert!(text.contains("/api/invites/{id}"), "{text}");
    assert!(text.contains("/api/reset"), "{text}");
    assert!(!text.contains(&reset_code), "reset code logged:\n{text}");
    assert!(!text.contains(&admin), "session logged:\n{text}");
    assert!(!text.contains(NEW_PASSWORD) && !text.contains(PASSWORD));
    assert!(
        !text.contains(&m.scorer_token),
        "scorer token logged:\n{text}"
    );
    assert!(text.contains("<static>"), "{text}");
}
