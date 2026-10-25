#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The player API (`/api/players`), through the real router.

mod support;

use axum::http::{Method, StatusCode, header};
use serde_json::json;
use support::*;

#[tokio::test]
async fn players_need_session() {
    let (state, _pool, app, admin_id, _raw) = with_admin().await;
    let res = send(&app, req(Method::GET, "/api/players", None, &[])).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let token = device_token(&state.store, &admin_id).await;
    let res = send(&app, as_device(&token, Method::GET, "/api/players", None)).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = send(
        &app,
        as_device(
            &token,
            Method::POST,
            "/api/players",
            Some(json!({"name": "Rui"})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_list_rename_archive_round_trip() {
    let (state, _pool, app, _id, raw) = with_admin().await;
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": "  Rui "})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    assert_eq!(res.body["name"], "Rui");
    assert_eq!(res.body["archived"], false);
    let pid = res.body["id"].as_str().unwrap().to_owned();

    let list = send(
        &app,
        as_session(&state, &raw, Method::GET, "/api/players", None),
    )
    .await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["items"].as_array().unwrap().len(), 1);
    assert_eq!(list.body["items"][0]["id"], pid.as_str());

    let path = format!("/api/players/{pid}");
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::PATCH,
            &path,
            Some(json!({"name": "Rui S"})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "Rui S");
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::PATCH,
            &path,
            Some(json!({"archived": true})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["archived"], true);
    assert_eq!(res.body["name"], "Rui S");

    // Renaming onto another player's name is a conflict.
    send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": "Marta"})),
        ),
    )
    .await;
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::PATCH,
            &path,
            Some(json!({"name": "marta"})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({"error": "player_exists"}));
}

#[tokio::test]
async fn create_existing_is_200_same_id() {
    let (state, _pool, app, _id, raw) = with_admin().await;
    let first = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": "Ana"})),
        ),
    )
    .await;
    assert_eq!(first.status, StatusCode::CREATED);
    let again = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": " ANA"})),
        ),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["id"], first.body["id"]);
}

#[tokio::test]
async fn blank_name_is_422_invalid_player() {
    let (state, _pool, app, _id, raw) = with_admin().await;
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": "   "})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body, json!({"error": "invalid_player"}));
    let long = "x".repeat(41);
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": long})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn other_users_player_is_404() {
    let (state, _pool, app, _id, raw_a) = with_admin().await;
    let (_b, raw_b) = invited_user(&state, &app, &raw_a, "b@x.test", "Rui").await;
    let made = send(
        &app,
        as_session(
            &state,
            &raw_a,
            Method::POST,
            "/api/players",
            Some(json!({"name": "João"})),
        ),
    )
    .await;
    let path = format!("/api/players/{}", made.body["id"].as_str().unwrap());
    let res = send(
        &app,
        as_session(
            &state,
            &raw_b,
            Method::PATCH,
            &path,
            Some(json!({"archived": true})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body, json!({"error": "not_found"}));
    let list = send(
        &app,
        as_session(&state, &raw_b, Method::GET, "/api/players", None),
    )
    .await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn cross_site_post_is_refused() {
    let (state, _pool, app, _id, raw) = with_admin().await;
    let cookie = cookie(&state, &raw);
    let request = axum::http::Request::builder()
        .method(Method::POST)
        .uri("/api/players")
        .header("sec-fetch-site", "cross-site")
        .header("cookie", cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(json!({"name": "Eve"}).to_string()))
        .unwrap();
    let res = send(&app, request).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    let list = send(
        &app,
        as_session(&state, &raw, Method::GET, "/api/players", None),
    )
    .await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 0);
}
