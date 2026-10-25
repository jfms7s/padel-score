#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Invite links through the real router: create, list, revoke, the public lookup and accept.

mod support;

use axum::http::{Method, StatusCode, header};
use padel_score_server::accounts_store::{INVITE_TTL_SECS, MAX_ACTIVE_INVITES};
use padel_score_server::api::now;
use serde_json::{Value, json};
use support::*;

async fn lookup(app: &axum::Router, code: &str) -> Res {
    let body = json!({ "code": code });
    send(
        app,
        req(Method::POST, "/api/invites/lookup", Some(body), &[]),
    )
    .await
}

async fn list(state: &padel_score_server::AppState, app: &axum::Router, raw: &str) -> Res {
    send(
        app,
        as_session(state, raw, Method::GET, "/api/invites", None),
    )
    .await
}

#[tokio::test]
async fn invite_url_carries_code_only_in_fragment() {
    let (state, _, app, _, admin) = with_admin().await;
    let before = now();
    let res = send(
        &app,
        as_session(&state, &admin, Method::POST, "/api/invites", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let obj = res.body.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["expires_at", "id", "url"]);
    let url = res.body["url"].as_str().unwrap();
    let (path, code) = url.split_once('#').unwrap();
    assert_eq!(path, "/invite");
    assert!(code.len() >= 40, "{code}");
    assert!(!path.contains(code) && !path.contains('?'));
    // No other field carries the code.
    let rest = json!({ "id": res.body["id"], "expires_at": res.body["expires_at"] });
    assert!(!rest.to_string().contains(code));
    assert!(res.headers.get(header::LOCATION).is_none());
    let expires_at = res.body["expires_at"].as_i64().unwrap();
    assert!((before + INVITE_TTL_SECS..=now() + INVITE_TTL_SECS).contains(&expires_at));
    // The code works.
    assert_eq!(lookup(&app, code).await.status, StatusCode::OK);
}

#[tokio::test]
async fn invite_list_never_returns_codes() {
    let (state, _, app, _, admin) = with_admin().await;
    let (used_id, used_code) = create_invite(&state, &app, &admin).await;
    let res = accept(&app, &used_code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let (revoked_id, revoked_code) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{revoked_id}");
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let (active_id, active_code) = create_invite(&state, &app, &admin).await;

    let res = list(&state, &app, &admin).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let text = res.body.to_string();
    for code in [&used_code, &revoked_code, &active_code] {
        assert!(!text.contains(code.as_str()), "{text}");
    }
    let items = res.body.as_array().unwrap();
    assert_eq!(items.len(), 3);
    let by_id = |id: &str| -> &Value { items.iter().find(|i| i["id"] == json!(id)).unwrap() };
    assert_eq!(by_id(&active_id)["status"], json!("active"));
    assert_eq!(by_id(&revoked_id)["status"], json!("revoked"));
    let used = by_id(&used_id);
    assert_eq!(used["status"], json!("used"));
    assert_eq!(used["used_by_name"], json!("Bea"));
    assert!(by_id(&active_id).get("used_by_name").is_none());
    for item in items {
        let mut keys: Vec<&str> = item
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys.retain(|k| *k != "used_by_name");
        assert_eq!(keys, ["created_at", "expires_at", "id", "status"]);
    }
}

#[tokio::test]
async fn revoke_own_invite_only() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;
    let (id, code) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{id}");
    // Someone else's invite reads as unknown, and stays valid.
    let res = send(&app, as_session(&state, &bea, Method::DELETE, &path, None)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body, json!({ "error": "not_found" }));
    assert_eq!(lookup(&app, &code).await.status, StatusCode::OK);
    let res = send(
        &app,
        as_session(&state, &bea, Method::DELETE, "/api/invites/nope", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // The creator's revoke works, and the code stops working.
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(lookup(&app, &code).await.status, StatusCode::GONE);
    // A used invite cannot be revoked.
    let (used_id, used_code) = create_invite(&state, &app, &admin).await;
    let res = accept(&app, &used_code, accept_body("cy@x.test", "Cy")).await;
    assert_eq!(res.status, StatusCode::CREATED);
    let path = format!("/api/invites/{used_id}");
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "invite_used" }));
    // Without a session: 401.
    let res = send(&app, req(Method::DELETE, &path, None, &[])).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invite_lookup_invalid_states_look_identical() {
    let (state, pool, app, _, admin) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &admin, "bea@x.test", "Bea").await;

    let (_, valid) = create_invite(&state, &app, &admin).await;
    let res = lookup(&app, &valid).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let expires_at = res.body["expires_at"].as_i64().unwrap();
    assert_eq!(
        res.body,
        json!({ "invited_by_name": "Ana", "expires_at": expires_at })
    );

    let (_, used) = create_invite(&state, &app, &admin).await;
    assert_eq!(
        accept(&app, &used, accept_body("cy@x.test", "Cy"))
            .await
            .status,
        StatusCode::CREATED
    );
    let (revoked_id, revoked) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{revoked_id}");
    send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    let (expired_id, expired) = create_invite(&state, &app, &admin).await;
    sqlx::query("UPDATE invites SET expires_at = ? WHERE id = ?")
        .bind(now() - 1)
        .bind(&expired_id)
        .execute(&pool)
        .await
        .unwrap();
    // An invite by a creator who is then disabled.
    let (_, by_disabled) = create_invite(&state, &app, &bea).await;
    state
        .store
        .set_disabled(&bea_id, Some(now()))
        .await
        .unwrap();

    let unknown = "x".repeat(43);
    let mut answers = Vec::new();
    for code in [&unknown, &used, &revoked, &expired, &by_disabled] {
        let res = lookup(&app, code).await;
        let mut headers: Vec<String> = res
            .headers
            .iter()
            .filter(|(n, _)| *n != header::DATE)
            .map(|(n, v)| format!("{n}: {}", v.to_str().unwrap()))
            .collect();
        headers.sort();
        answers.push((res.status, res.body, headers));
    }
    let first = answers[0].clone();
    assert_eq!(first.0, StatusCode::GONE);
    assert_eq!(first.1, json!({ "error": "invite_invalid" }));
    for answer in &answers {
        assert_eq!(answer, &first);
    }
    // Accepting any of them is the same 410, and makes no user.
    let users = state.store.user_count().await.unwrap();
    for (i, code) in [&unknown, &used, &revoked, &expired, &by_disabled]
        .iter()
        .enumerate()
    {
        let res = accept(&app, code, accept_body(&format!("n{i}@x.test"), "N")).await;
        assert_eq!(res.status, StatusCode::GONE, "{code}");
        assert_eq!(res.body, json!({ "error": "invite_invalid" }));
    }
    assert_eq!(state.store.user_count().await.unwrap(), users);
}

#[tokio::test]
async fn accept_creates_user_signed_in_with_inviter() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let (_, code) = create_invite(&state, &app, &admin).await;
    // Validation names the field, as setup does, and leaves the invite usable.
    for (body, field) in [
        (
            json!({ "email": "nope", "display_name": "Bea", "password": PASSWORD }),
            "email",
        ),
        (
            json!({ "email": "bea@x.test", "display_name": " ", "password": PASSWORD }),
            "display_name",
        ),
        (
            json!({ "email": "bea@x.test", "display_name": "Bea", "password": "short" }),
            "password",
        ),
    ] {
        let res = accept(&app, &code, body).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(res.body, json!({ "error": "validation", "field": field }));
    }
    // Accepted in a browser where the admin is signed in: that session ends.
    let mut body = accept_body(" Bea@X.test ", " Bea ");
    body["code"] = json!(code);
    let res = send(
        &app,
        as_session(
            &state,
            &admin,
            Method::POST,
            "/api/invites/accept",
            Some(body),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["id"].as_str().unwrap().to_owned();
    assert_eq!(
        res.body,
        json!({ "id": id, "email": "bea@x.test", "display_name": "Bea", "is_admin": false })
    );
    let raw = res.session(&state).unwrap();
    assert!(!raw.is_empty());
    let res = me(&state, &app, &raw).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["id"], json!(id));
    assert_eq!(
        me(&state, &app, &admin).await.status,
        StatusCode::UNAUTHORIZED
    );
    let user = state.store.user_by_id(&id).await.unwrap().unwrap();
    assert_eq!(user.invited_by.as_deref(), Some(admin_id.as_str()));
    assert!(!user.is_admin);
    // The new user can sign in with the password.
    assert_eq!(
        login(&app, "bea@x.test", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn invite_cannot_be_used_twice() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, code) = create_invite(&state, &app, &admin).await;
    let res = accept(&app, &code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::CREATED);
    let res = accept(&app, &code, accept_body("cy@x.test", "Cy")).await;
    assert_eq!(res.status, StatusCode::GONE);
    assert_eq!(res.body, json!({ "error": "invite_invalid" }));
    assert!(res.session(&state).is_none());
    assert_eq!(state.store.user_count().await.unwrap(), 2);
    assert!(
        state
            .store
            .user_by_email("cy@x.test")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn accept_duplicate_email_is_409() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, code) = create_invite(&state, &app, &admin).await;
    let res = accept(&app, &code, accept_body(" ADMIN@x.test", "Other")).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "email_taken" }));
    assert!(res.session(&state).is_none());
    // The invite stays usable.
    assert_eq!(lookup(&app, &code).await.status, StatusCode::OK);
    let res = accept(&app, &code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::CREATED);
}

#[tokio::test]
async fn twenty_active_invites_limit() {
    let (state, _, app, _, admin) = with_admin().await;
    let mut first = None;
    for _ in 0..MAX_ACTIVE_INVITES {
        let made = create_invite(&state, &app, &admin).await;
        first.get_or_insert(made);
    }
    let res = send(
        &app,
        as_session(&state, &admin, Method::POST, "/api/invites", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "too_many_invites" }));
    // Revoking one frees a slot.
    let (id, _) = first.unwrap();
    let path = format!("/api/invites/{id}");
    let res = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    create_invite(&state, &app, &admin).await;
}

#[tokio::test]
async fn invite_lookup_is_rate_limited() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, code) = create_invite(&state, &app, &admin).await;
    for i in 0..120 {
        let res = lookup(&app, &code).await;
        assert_eq!(res.status, StatusCode::OK, "lookup {i}");
    }
    let res = lookup(&app, &code).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    assert!(res.header(header::RETRY_AFTER).is_some());
    // Accept shares the address's budget.
    let res = accept(&app, &code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    // Another address is not limited.
    let other = app_from(&state, 2);
    assert_eq!(lookup(&other, &code).await.status, StatusCode::OK);
    let res = accept(&other, &code, accept_body("bea@x.test", "Bea")).await;
    assert_eq!(res.status, StatusCode::CREATED);
}

#[tokio::test]
async fn device_token_cannot_create_invites() {
    let (state, _, app, admin_id, admin) = with_admin().await;
    let token = device_token(&state.store, &admin_id).await;
    let (id, _) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{id}");
    for (method, path) in [
        (Method::POST, "/api/invites"),
        (Method::GET, "/api/invites"),
        (Method::DELETE, path.as_str()),
    ] {
        let res = send(&app, as_device(&token, method.clone(), path, None)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(res.body, json!({ "error": "forbidden" }));
    }
    // The device token is otherwise good.
    let res = send(&app, as_device(&token, Method::GET, "/api/auth/me", None)).await;
    assert_eq!(res.status, StatusCode::OK);
    let listed = list(&state, &app, &admin).await;
    assert_eq!(listed.body[0]["status"], json!("active"));
}

#[tokio::test]
async fn invite_responses_no_store_no_referrer() {
    let (state, _, app, _, admin) = with_admin().await;
    let created = send(
        &app,
        as_session(&state, &admin, Method::POST, "/api/invites", None),
    )
    .await;
    let code = fragment_code(created.body["url"].as_str().unwrap());
    let id = created.body["id"].as_str().unwrap().to_owned();
    let listed = list(&state, &app, &admin).await;
    let found = lookup(&app, &code).await;
    let missing = lookup(&app, "unknown-code").await;
    let invalid = accept(&app, "unknown-code", accept_body("bea@x.test", "Bea")).await;
    let accepted = accept(&app, &code, accept_body("bea@x.test", "Bea")).await;
    let (other_id, _) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{other_id}");
    let revoked = send(
        &app,
        as_session(&state, &admin, Method::DELETE, &path, None),
    )
    .await;
    assert_ne!(id, other_id);
    for (name, res) in [
        ("create", &created),
        ("list", &listed),
        ("lookup", &found),
        ("lookup missing", &missing),
        ("accept invalid", &invalid),
        ("accept", &accepted),
        ("revoke", &revoked),
    ] {
        assert_eq!(
            res.header(header::REFERRER_POLICY),
            Some("no-referrer"),
            "{name}"
        );
        assert_eq!(
            res.header(header::CACHE_CONTROL),
            Some("no-store"),
            "{name}"
        );
    }
}

/// Codes travel in bodies only: the old path-carrying routes are gone.
#[tokio::test]
async fn code_in_path_routes_are_gone() {
    let (state, _, app, _, admin) = with_admin().await;
    let (_, code) = create_invite(&state, &app, &admin).await;
    let path = format!("/api/invites/{code}");
    let res = send(&app, req(Method::GET, &path, None, &[])).await;
    assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
    let path = format!("/api/invites/{code}/accept");
    let body = accept_body("bea@x.test", "Bea");
    let res = send(&app, req(Method::POST, &path, Some(body), &[])).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // The invite is still usable through the body routes.
    assert_eq!(lookup(&app, &code).await.status, StatusCode::OK);
}
