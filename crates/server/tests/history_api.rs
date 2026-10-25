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

fn match_body(teams: serde_json::Value, players: Option<serde_json::Value>) -> serde_json::Value {
    let mut body = json!({
        "format": {"sets": {"deuce": "advantage", "sets": "one", "deciding": "full_set"}},
        "start": {"first_server": {"team": "a", "index": 0}, "other_first_server": 0},
        "teams": teams,
    });
    if let Some(players) = players {
        body["players"] = players;
    }
    body
}

fn free_teams() -> serde_json::Value {
    json!({"a": ["Xx", "Yy"], "b": ["Zz", "Ww"]})
}

async fn make_player(
    state: &padel_score_server::AppState,
    app: &axum::Router,
    raw: &str,
    name: &str,
) -> String {
    let res = send(
        app,
        as_session(
            state,
            raw,
            Method::POST,
            "/api/players",
            Some(json!({"name": name})),
        ),
    )
    .await;
    res.body["id"].as_str().unwrap().to_owned()
}

async fn create(
    state: &padel_score_server::AppState,
    app: &axum::Router,
    raw: Option<&str>,
    body: serde_json::Value,
) -> support::Res {
    match raw {
        Some(raw) => {
            send(
                app,
                as_session(state, raw, Method::POST, "/api/matches", Some(body)),
            )
            .await
        }
        None => send(app, req(Method::POST, "/api/matches", Some(body), &[])).await,
    }
}

async fn count(pool: &sqlx::SqlitePool, table: &str) -> i64 {
    let sql = match table {
        "matches" => "SELECT COUNT(*) FROM matches",
        "match_players" => "SELECT COUNT(*) FROM match_players",
        other => panic!("count: unknown table {other}"),
    };
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn match_with_players_records_slots() {
    let (state, pool, app, _id, raw) = with_admin().await;
    let ana = make_player(&state, &app, &raw, "Ana").await;
    let rui = make_player(&state, &app, &raw, "Rui").await;
    let body = match_body(
        free_teams(),
        Some(json!({"a": [ana, null], "b": [null, rui]})),
    );
    let res = create(&state, &app, Some(&raw), body).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT team, position, player_id FROM match_players ORDER BY team, position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("a".into(), 0, ana.clone()), ("b".into(), 1, rui.clone())]
    );
    let teams: String = sqlx::query_scalar("SELECT teams FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    let teams: serde_json::Value = serde_json::from_str(&teams).unwrap();
    assert_eq!(teams, json!({"a": ["Ana", "Yy"], "b": ["Zz", "Rui"]}));
}

#[tokio::test]
async fn match_with_players_needs_account() {
    let (state, pool, app, _id, raw) = with_admin().await;
    let ana = make_player(&state, &app, &raw, "Ana").await;
    let body = match_body(free_teams(), Some(json!({"a": [ana, null]})));
    let res = create(&state, &app, None, body).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["error"], "invalid_match");
    assert_eq!(count(&pool, "matches").await, 0);
}

#[tokio::test]
async fn match_with_other_users_player_is_422() {
    let (state, pool, app, _id, raw_a) = with_admin().await;
    let (_b, raw_b) = invited_user(&state, &app, &raw_a, "b@x.test", "Rui").await;
    let ana = make_player(&state, &app, &raw_a, "Ana").await;
    let foreign = create(
        &state,
        &app,
        Some(&raw_b),
        match_body(free_teams(), Some(json!({"a": [ana, null]}))),
    )
    .await;
    let missing = create(
        &state,
        &app,
        Some(&raw_b),
        match_body(free_teams(), Some(json!({"a": ["nope", null]}))),
    )
    .await;
    assert_eq!(foreign.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(foreign.status, missing.status);
    assert_eq!(foreign.body, missing.body);
    assert_eq!(count(&pool, "matches").await, 0);
}

#[tokio::test]
async fn match_with_archived_player_is_422() {
    let (state, pool, app, _id, raw) = with_admin().await;
    let ana = make_player(&state, &app, &raw, "Ana").await;
    let res = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::PATCH,
            &format!("/api/players/{ana}"),
            Some(json!({"archived": true})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = create(
        &state,
        &app,
        Some(&raw),
        match_body(free_teams(), Some(json!({"a": [ana, null]}))),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(count(&pool, "matches").await, 0);
}

#[tokio::test]
async fn same_player_twice_is_422() {
    let (state, pool, app, _id, raw) = with_admin().await;
    let ana = make_player(&state, &app, &raw, "Ana").await;
    let res = create(
        &state,
        &app,
        Some(&raw),
        match_body(
            free_teams(),
            Some(json!({"a": [ana, null], "b": [ana, null]})),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["error"], "invalid_match");
    assert_eq!(count(&pool, "matches").await, 0);
    assert_eq!(count(&pool, "match_players").await, 0);
}

#[tokio::test]
async fn mixed_slots_are_fine() {
    let (state, pool, app, _id, raw) = with_admin().await;
    let ana = make_player(&state, &app, &raw, "Ana").await;
    let res = create(
        &state,
        &app,
        Some(&raw),
        match_body(free_teams(), Some(json!({"b": [null, ana]}))),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(count(&pool, "match_players").await, 1);
}

#[tokio::test]
async fn anonymous_match_unchanged() {
    let (state, pool, app, _id, _raw) = with_admin().await;
    let res = create(&state, &app, None, match_body(free_teams(), None)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(count(&pool, "matches").await, 1);
    assert_eq!(count(&pool, "match_players").await, 0);
}
