//! The coordinator's event routes and the live event stream (phase 6, Task 6).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use padel_score_server::AppState;
use padel_score_server::hub::{CourtPhones, Hub};
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use support::{
    Res, Sse, as_device, as_session, cookie, device_token, invited_user, req, send, with_admin,
};

fn one_set() -> Value {
    serde_json::to_value(Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    }))
    .unwrap()
}

fn new_event(name: &str) -> Value {
    json!({ "name": name, "kind": "tournament", "format": one_set() })
}

async fn call(
    state: &AppState,
    app: &Router,
    raw: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Res {
    send(app, as_session(state, raw, method, path, body)).await
}

async fn player(state: &AppState, app: &Router, raw: &str, name: &str) -> String {
    let res = call(
        state,
        app,
        raw,
        Method::POST,
        "/api/players",
        Some(json!({ "name": name })),
    )
    .await;
    assert!(res.status.is_success(), "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

async fn create_event(state: &AppState, app: &Router, raw: &str, name: &str) -> String {
    let res = call(
        state,
        app,
        raw,
        Method::POST,
        "/api/events",
        Some(new_event(name)),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["event"]["status"], json!("setup"));
    res.body["event"]["id"].as_str().unwrap().to_owned()
}

/// Starting a tournament draws its bracket and puts matches on the courts. These tests are about
/// courts and hand-made event matches, so remove the drawn matches (their courts go free).
async fn clear_draw(pool: &sqlx::SqlitePool) {
    sqlx::query("DELETE FROM matches WHERE id IN (SELECT match_id FROM bracket_matches)")
        .execute(pool)
        .await
        .unwrap();
}

/// A "Sunday cup" of the session `raw` with four teams and two courts, started, and one match
/// created on court 1: (event id, team ids, court ids, match id).
async fn running_event(
    state: &AppState,
    pool: &sqlx::SqlitePool,
    app: &Router,
    raw: &str,
) -> (String, Vec<String>, Vec<String>, String) {
    let id = create_event(state, app, raw, "Sunday cup").await;
    let mut teams = Vec::new();
    for [p1, p2] in [
        ["Ana", "Marta"],
        ["Rui", "João"],
        ["Eva", "Inês"],
        ["Tiago", "Luís"],
    ] {
        let players = [
            player(state, app, raw, p1).await,
            player(state, app, raw, p2).await,
        ];
        let res = call(
            state,
            app,
            raw,
            Method::POST,
            &format!("/api/events/{id}/teams"),
            Some(json!({ "players": players })),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert_eq!(res.body["name"], json!(format!("{p1} / {p2}")));
        teams.push(res.body["id"].as_str().unwrap().to_owned());
    }
    let mut courts = Vec::new();
    for _ in 0..2 {
        let res = call(
            state,
            app,
            raw,
            Method::POST,
            &format!("/api/events/{id}/courts"),
            Some(json!({})),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
        assert_eq!(res.body["phones"], json!(0));
        assert!(res.body["scorer_token"].is_string());
        courts.push(res.body["id"].as_str().unwrap().to_owned());
    }
    let res = call(
        state,
        app,
        raw,
        Method::POST,
        &format!("/api/events/{id}/start"),
        None,
    )
    .await;
    clear_draw(pool).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["status"], json!("running"));
    let res = call(
        state,
        app,
        raw,
        Method::POST,
        &format!("/api/events/{id}/matches"),
        Some(json!({ "team_a": teams[0], "team_b": teams[1], "court_id": courts[0] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let match_id = res.body["id"].as_str().unwrap().to_owned();
    (id, teams, courts, match_id)
}

async fn detail(state: &AppState, app: &Router, raw: &str, id: &str) -> Value {
    let res = call(
        state,
        app,
        raw,
        Method::GET,
        &format!("/api/events/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    assert_eq!(res.header(header::REFERRER_POLICY), Some("no-referrer"));
    res.body
}

fn match_in<'a>(detail: &'a Value, match_id: &str) -> &'a Value {
    detail["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(match_id))
        .unwrap()
}

fn court_in<'a>(detail: &'a Value, court_id: &str) -> &'a Value {
    detail["courts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(court_id))
        .unwrap()
}

fn point(client_id: &str) -> Value {
    json!({
        "client_id": client_id, "batch": false, "base_revision": 0,
        "commands": [{ "kind": "point", "team": "a" }],
    })
}

#[tokio::test]
async fn events_need_session() {
    let (state, _pool, app, admin, _raw) = with_admin().await;
    let token = device_token(&state.store, &admin).await;
    let anon = send(&app, req(Method::GET, "/api/events", None, &[])).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let anon = send(
        &app,
        req(Method::POST, "/api/events", Some(new_event("Cup")), &[]),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let device = send(&app, as_device(&token, Method::GET, "/api/events", None)).await;
    assert_eq!(device.status, StatusCode::FORBIDDEN);
    let device = send(
        &app,
        as_device(&token, Method::POST, "/api/events", Some(new_event("Cup"))),
    )
    .await;
    assert_eq!(device.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_event_round_trip() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, teams, courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(d["event"]["name"], json!("Sunday cup"));
    assert_eq!(d["event"]["team_count"], json!(4));
    assert_eq!(d["event"]["court_count"], json!(2));
    assert!(d["event"].get("owner_user_id").is_none());
    assert!(d["spectator_token"].is_string());
    assert_eq!(d["teams"].as_array().unwrap().len(), 4);
    assert_eq!(d["kind_data"], Value::Null);
    assert_eq!(d["places"], json!([]));
    assert_eq!(d["places_computed"], json!(false));
    let court = court_in(&d, &courts[0]);
    assert_eq!(court["name"], json!("Court 1"));
    assert_eq!(court["current"], json!(match_id));
    assert_eq!(court["queue"], json!([]));
    assert_eq!(court["phones"], json!(0));
    let m = match_in(&d, &match_id);
    assert_eq!(m["state"], json!("waiting"));
    assert_eq!(m["label"], json!("Match 1"));
    assert_eq!(m["team_a"], json!(teams[0]));
    assert_eq!(m["court_id"], json!(courts[0]));
    assert_eq!(m["locked"], json!(false));
    assert_eq!(m["live"], Value::Null);
    assert_eq!(m["result"], Value::Null);
    assert!(m["scorer_token"].is_string());

    // A second match queues behind it; the list has the event.
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/matches"),
        Some(json!({ "team_a": teams[2], "team_b": teams[3], "court_id": courts[0], "label": "Semi" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["label"], json!("Semi"));
    let second = res.body["id"].as_str().unwrap().to_owned();
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(court_in(&d, &courts[0])["queue"], json!([second]));
    // Moving it to the idle court makes it current there.
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}/matches/{second}"),
        Some(json!({ "court_id": courts[1] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["court_id"], json!(courts[1]));
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(court_in(&d, &courts[1])["current"], json!(second));

    let list = call(&state, &app, &raw, Method::GET, "/api/events", None).await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["items"][0]["id"], json!(id));
    assert_eq!(list.body["items"][0]["team_count"], json!(4));

    // An unknown match team is 422 teams; matches before start are 409.
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/matches"),
        Some(json!({ "team_a": teams[0], "team_b": "nope" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body, json!({ "error": "validation", "field": "teams" }));
    let other = create_event(&state, &app, &raw, "Later").await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{other}/matches"),
        Some(json!({ "team_a": teams[0], "team_b": teams[1] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("event_not_running"));
}

#[tokio::test]
async fn create_validation_errors() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        "/api/events",
        Some(new_event("  ")),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body, json!({ "error": "validation", "field": "name" }));
    let mut timed = new_event("Cup");
    timed["kind"] = json!("non_stop");
    let res = call(&state, &app, &raw, Method::POST, "/api/events", Some(timed)).await;
    assert_eq!(
        res.body,
        json!({ "error": "validation", "field": "format" })
    );
    let mut settings = new_event("Cup");
    settings["settings"] = json!({ "x": 1 });
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        "/api/events",
        Some(settings),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["field"], json!("settings"));
    // Too few teams to start.
    let id = create_event(&state, &app, &raw, "Cup").await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/start"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("too_few_teams"));
    // Three players in a team is 422 players, not a 400.
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/teams"),
        Some(json!({ "players": ["a", "b", "c"] })),
    )
    .await;
    assert_eq!(
        res.body,
        json!({ "error": "validation", "field": "players" })
    );
    // Rename via PATCH works in setup.
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}"),
        Some(json!({ "name": "Big cup" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["name"], json!("Big cup"));
    // Delete in setup is 204, then 404.
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = call(
        &state,
        &app,
        &raw,
        Method::GET,
        &format!("/api/events/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn team_and_court_edits() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = create_event(&state, &app, &raw, "Cup").await;
    let players = [
        player(&state, &app, &raw, "Ana").await,
        player(&state, &app, &raw, "Rui").await,
    ];
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/teams"),
        Some(json!({ "players": players, "name": "Lions" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["custom_name"], json!("Lions"));
    let team = res.body["id"].as_str().unwrap().to_owned();
    // `name: null` clears the custom name.
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}/teams/{team}"),
        Some(json!({ "name": null })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["custom_name"], Value::Null);
    assert_eq!(res.body["name"], json!("Ana / Rui"));
    // The same player twice in the event is 409.
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/teams"),
        Some(json!({ "players": [players[0], player(&state, &app, &raw, "Eva").await] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("player_in_team"));
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}/teams/{team}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/courts"),
        Some(json!({ "name": "Centre" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED);
    let court = res.body["id"].as_str().unwrap().to_owned();
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}/courts/{court}"),
        Some(json!({ "name": "" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}/courts/{court}"),
        Some(json!({ "name": "Court A" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["name"], json!("Court A"));
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}/courts/{court}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn other_users_event_is_404() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (_b, braw) = invited_user(&state, &app, &raw, "b@x.test", "Rui").await;
    let (id, teams, courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let bp = [
        player(&state, &app, &braw, "Eva").await,
        player(&state, &app, &braw, "Inês").await,
    ];
    let attempts: Vec<(Method, String, Option<Value>)> = vec![
        (Method::GET, format!("/api/events/{id}"), None),
        (
            Method::PATCH,
            format!("/api/events/{id}"),
            Some(json!({ "name": "Mine" })),
        ),
        (Method::DELETE, format!("/api/events/{id}"), None),
        (
            Method::POST,
            format!("/api/events/{id}/teams"),
            Some(json!({ "players": bp })),
        ),
        (
            Method::POST,
            format!("/api/events/{id}/courts"),
            Some(json!({})),
        ),
        (
            Method::POST,
            format!("/api/events/{id}/courts/{}/rotate", courts[0]),
            None,
        ),
        (
            Method::POST,
            format!("/api/events/{id}/matches"),
            Some(json!({ "team_a": teams[0], "team_b": teams[1] })),
        ),
        (
            Method::POST,
            format!("/api/events/{id}/matches/{match_id}/result"),
            Some(
                json!({ "client_id": "b1", "score": { "sets": [{ "a": 6, "b": 2, "tiebreak": null }] } }),
            ),
        ),
        (Method::GET, format!("/api/events/{id}/stream"), None),
        (Method::POST, format!("/api/events/{id}/start"), None),
        (
            Method::POST,
            format!("/api/events/{id}/finish"),
            Some(json!({ "places": [
                { "team_id": teams[0], "place": 1, "place_to": 1 },
                { "team_id": teams[1], "place": 2, "place_to": 2 },
                { "team_id": teams[2], "place": 3, "place_to": 3 },
                { "team_id": teams[3], "place": 4, "place_to": 4 },
            ] })),
        ),
        (
            Method::POST,
            format!("/api/events/{id}/courts/{}/advance", courts[0]),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/events/{id}/teams/{}", teams[0]),
            Some(json!({ "name": "Mine" })),
        ),
        (
            Method::DELETE,
            format!("/api/events/{id}/teams/{}", teams[0]),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/events/{id}/courts/{}", courts[0]),
            Some(json!({ "name": "Mine" })),
        ),
        (
            Method::DELETE,
            format!("/api/events/{id}/courts/{}", courts[1]),
            None,
        ),
        (
            Method::PATCH,
            format!("/api/events/{id}/matches/{match_id}"),
            Some(json!({ "court_id": courts[1], "label": "Mine" })),
        ),
        (Method::GET, "/api/events/nope".to_owned(), None),
    ];
    for (method, path, body) in attempts {
        let res = call(&state, &app, &braw, method.clone(), &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert_eq!(res.body, json!({ "error": "not_found" }));
    }
    // B's list is empty, and A's event is untouched.
    let list = call(&state, &app, &braw, Method::GET, "/api/events", None).await;
    assert_eq!(list.body["items"], json!([]));
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(d["event"]["name"], json!("Sunday cup"));
    assert_eq!(d["event"]["status"], json!("running"));
    assert_eq!(d["teams"].as_array().unwrap().len(), 4);
    assert_eq!(d["teams"][0]["custom_name"], Value::Null);
    assert_eq!(d["courts"].as_array().unwrap().len(), 2);
    assert_eq!(court_in(&d, &courts[0])["name"], json!("Court 1"));
    assert_eq!(court_in(&d, &courts[0])["current"], json!(match_id));
    let m = match_in(&d, &match_id);
    assert_eq!(m["state"], json!("waiting"));
    assert_eq!(m["label"], json!("Match 1"));
    assert_eq!(m["court_id"], json!(courts[0]));
    assert_eq!(d["places"], json!([]));
}

#[tokio::test]
async fn team_with_other_users_player_is_422() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let (_b, braw) = invited_user(&state, &app, &raw, "b@x.test", "Rui").await;
    let id = create_event(&state, &app, &raw, "Cup").await;
    let mine = player(&state, &app, &raw, "Ana").await;
    let theirs = player(&state, &app, &braw, "Marta").await;
    let path = format!("/api/events/{id}/teams");
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "players": [mine, theirs] })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        res.body,
        json!({ "error": "validation", "field": "players" })
    );
    let unknown = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "players": [mine, "no-such-player"] })),
    )
    .await;
    assert_eq!(unknown.status, res.status);
    assert_eq!(unknown.body, res.body);
}

#[tokio::test]
async fn cross_site_post_is_refused() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/events")
        .header("sec-fetch-site", "cross-site")
        .header("cookie", cookie(&state, &raw))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(new_event("Cup").to_string()))
        .unwrap();
    let res = send(&app, request).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let list = call(&state, &app, &raw, Method::GET, "/api/events", None).await;
    assert_eq!(list.body["items"], json!([]));
}

#[tokio::test]
async fn coordinator_result_locks_and_shows() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let path = format!("/api/events/{id}/matches/{match_id}/result");
    let body =
        json!({ "client_id": "r1", "score": { "sets": [{ "a": 6, "b": 2, "tiebreak": null }] } });
    let res = call(&state, &app, &raw, Method::POST, &path, Some(body.clone())).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["locked"], json!(true));
    // A resend replays.
    let again = call(&state, &app, &raw, Method::POST, &path, Some(body)).await;
    assert_eq!(again.status, StatusCode::OK);
    let d = detail(&state, &app, &raw, &id).await;
    let m = match_in(&d, &match_id);
    assert_eq!(m["state"], json!("finished"));
    assert_eq!(m["winner"], json!("a"));
    assert_eq!(m["locked"], json!(true));
    assert_eq!(
        m["result"]["sets"],
        json!([{ "a": 6, "b": 2, "tiebreak": null }])
    );
    assert_eq!(m["live"], Value::Null);
    // An impossible score is 422.
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "client_id": "r2", "score": { "sets": [{ "a": 6, "b": 5, "tiebreak": null }] } })),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(res.body["error"], json!("invalid_score"));
    // A finished match cannot be moved; the court advances.
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}/matches/{match_id}"),
        Some(json!({ "court_id": null })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("match_finished"));
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/courts/{}/advance", courts[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(court_in(&res.body, &courts[0])["current"], Value::Null);
}

#[tokio::test]
async fn advance_unfinished_is_409() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, courts, _match_id) = running_event(&state, &pool, &app, &raw).await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/courts/{}/advance", courts[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("match_not_finished"));
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}/courts/{}", courts[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("court_in_use"));
}

#[tokio::test]
async fn event_stream_updates_on_court_point() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, _courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let scorer = match_in(&d, &match_id)["scorer_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut stream = Sse::open(
        &app,
        as_session(
            &state,
            &raw,
            Method::GET,
            &format!("/api/events/{id}/stream"),
            None,
        ),
    )
    .await;
    let first = stream.next("event").await;
    assert_eq!(match_in(&first, &match_id)["state"], json!("waiting"));
    let res = send(
        &app,
        req(
            Method::POST,
            &format!("/api/score/{scorer}/commands"),
            Some(point("p1")),
            &[],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let next = stream.next("event").await;
    let m = match_in(&next, &match_id);
    assert_eq!(m["state"], json!("live"));
    assert_eq!(m["live"]["games"], json!([0, 0]));
    // A rename reaches the stream too.
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}"),
        Some(json!({ "name": "Monday cup" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK);
    let next = stream.next("event").await;
    assert_eq!(next["event"]["name"], json!("Monday cup"));
}

#[tokio::test]
async fn event_stream_ends_when_deleted() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = create_event(&state, &app, &raw, "Cup").await;
    let mut stream = Sse::open(
        &app,
        as_session(
            &state,
            &raw,
            Method::GET,
            &format!("/api/events/{id}/stream"),
            None,
        ),
    )
    .await;
    stream.next("event").await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(stream.ends_within(Duration::from_secs(2)).await);
}

#[tokio::test]
async fn rotate_ends_old_fence_stream() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let old = match_in(&d, &match_id)["scorer_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let old_court = court_in(&d, &courts[0])["scorer_token"].clone();
    let mut fence = Sse::open(
        &app,
        req(Method::GET, &format!("/api/score/{old}/stream"), None, &[]),
    )
    .await;
    fence.next("state").await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{id}/courts/{}/rotate", courts[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_ne!(res.body["scorer_token"], old_court);
    assert_eq!(res.body["current"], json!(match_id));
    assert!(fence.ends_within(Duration::from_secs(2)).await);
    let res = send(
        &app,
        req(Method::GET, &format!("/api/score/{old}"), None, &[]),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn finish_via_api_writes_placings() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, teams, courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let path = format!("/api/events/{id}/finish");
    let bad = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "places": [{ "team_id": teams[0], "place": 1, "place_to": 1 }] })),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        bad.body,
        json!({ "error": "validation", "field": "places" })
    );
    let places = json!([
        { "team_id": teams[0], "place": 1, "place_to": 1 },
        { "team_id": teams[1], "place": 2, "place_to": 2 },
        { "team_id": teams[2], "place": 3, "place_to": 4 },
        { "team_id": teams[3], "place": 3, "place_to": 4 },
    ]);
    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "places": places })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["status"], json!("finished"));
    assert_eq!(res.body["places"].as_array().unwrap().len(), 4);
    assert_eq!(res.body["places"][0]["team_id"], json!(teams[0]));
    assert_eq!(match_in(&res.body, &match_id)["court_id"], json!(courts[0]));
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM placings WHERE event_id = ?")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 8);
    let again = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &path,
        Some(json!({ "places": places })),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["error"], json!("event_not_running"));
}

#[tokio::test]
async fn delete_started_event_is_409() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, _courts, _match_id) = running_event(&state, &pool, &app, &raw).await;
    let res = call(
        &state,
        &app,
        &raw,
        Method::DELETE,
        &format!("/api/events/{id}"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "event_started" }));
    let res = call(
        &state,
        &app,
        &raw,
        Method::PATCH,
        &format!("/api/events/{id}"),
        Some(json!({ "format": one_set() })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], json!("event_started"));
}

#[tokio::test]
async fn live_list_has_court_context() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (_id, _teams, _courts, match_id) = running_event(&state, &pool, &app, &raw).await;
    let res = call(&state, &app, &raw, Method::GET, "/api/me/matches", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let item = res
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(match_id))
        .unwrap();
    assert_eq!(item["context"], json!("Court 1 · Sunday cup"));
    // A quick match has no context key.
    let created = send(
        &app,
        as_session(
            &state,
            &raw,
            Method::POST,
            "/api/matches",
            Some(json!({
                "format": one_set(),
                "start": { "first_server": { "team": "a", "index": 0 }, "other_first_server": 0 },
                "teams": { "a": ["Ana", "Rui"], "b": ["Eva", "Inês"] },
            })),
        ),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let res = call(&state, &app, &raw, Method::GET, "/api/me/matches", None).await;
    let quick = res
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == created.body["id"])
        .unwrap();
    assert!(quick.get("context").is_none());
}

#[tokio::test]
async fn court_phones_count_and_notify() {
    let hub = Arc::new(Hub::new());
    let phones = Arc::new(CourtPhones::new());
    let mut sub = hub.subscribe_event("ev1");
    assert_eq!(phones.count("c1"), 0);
    let one = phones.join(&hub, "ev1", "c1");
    let two = phones.join(&hub, "ev1", "c1");
    assert_eq!(phones.count("c1"), 2);
    assert_eq!(phones.count("c2"), 0);
    sub.try_recv().unwrap();
    sub.try_recv().unwrap();
    drop(one);
    assert_eq!(phones.count("c1"), 1);
    sub.try_recv().unwrap();
    drop(two);
    assert_eq!(phones.count("c1"), 0);
    sub.try_recv().unwrap();
    assert!(sub.try_recv().is_err());
}

#[tokio::test]
async fn detail_shows_phones() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let (id, _teams, courts, _match_id) = running_event(&state, &pool, &app, &raw).await;
    let guard = state.court_phones.join(&state.hub, &id, &courts[0]);
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(court_in(&d, &courts[0])["phones"], json!(1));
    assert_eq!(court_in(&d, &courts[1])["phones"], json!(0));
    drop(guard);
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(court_in(&d, &courts[0])["phones"], json!(0));
}

#[tokio::test]
async fn event_stream_ends_when_signed_out() {
    let (mut state, _pool, _app, _admin, raw) = with_admin().await;
    state.credential_recheck = Duration::from_millis(50);
    let app = support::app_from(&state, 1);
    let id = create_event(&state, &app, &raw, "Cup").await;
    let mut stream = Sse::open(
        &app,
        as_session(
            &state,
            &raw,
            Method::GET,
            &format!("/api/events/{id}/stream"),
            None,
        ),
    )
    .await;
    stream.next("event").await;
    assert!(!stream.ends_within(Duration::from_millis(200)).await);
    let res = call(&state, &app, &raw, Method::POST, "/api/auth/logout", None).await;
    assert!(res.status.is_success(), "{}", res.body);
    assert!(
        stream.ends_within(Duration::from_secs(2)).await,
        "stream outlived its session"
    );
}
