//! The tournament routes (redraw, decisions, fill courts) and the bracket in the event views
//! (phase 7, Task 4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode, header};
use padel_score_server::AppState;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use support::{Res, Sse, as_device, as_session, device_token, invited_user, req, send, with_admin};

fn one_set() -> Value {
    serde_json::to_value(Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    }))
    .unwrap()
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

async fn post(state: &AppState, app: &Router, raw: &str, path: &str, body: Option<Value>) -> Res {
    call(state, app, raw, Method::POST, path, body).await
}

/// An event of the session `raw` with `n` teams and `courts` courts, still in setup.
async fn setup_event(
    state: &AppState,
    app: &Router,
    raw: &str,
    new: Value,
    n: usize,
    courts: usize,
) -> String {
    let res = post(state, app, raw, "/api/events", Some(new)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    for i in 0..n {
        let mut players = Vec::new();
        for name in [format!("Ana{i}"), format!("Rui{i}")] {
            let res = post(
                state,
                app,
                raw,
                "/api/players",
                Some(json!({ "name": name })),
            )
            .await;
            assert!(res.status.is_success(), "{}", res.body);
            players.push(res.body["id"].as_str().unwrap().to_owned());
        }
        let res = post(
            state,
            app,
            raw,
            &format!("/api/events/{id}/teams"),
            Some(json!({ "players": players })),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    }
    for _ in 0..courts {
        add_court(state, app, raw, &id).await;
    }
    id
}

async fn add_court(state: &AppState, app: &Router, raw: &str, id: &str) -> String {
    let res = post(
        state,
        app,
        raw,
        &format!("/api/events/{id}/courts"),
        Some(json!({})),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

fn tournament(full_placement: bool) -> Value {
    json!({
        "name": "Sunday cup", "kind": "tournament", "format": one_set(),
        "settings": { "full_placement": full_placement },
    })
}

/// A started 8-team tournament on two courts.
async fn running(state: &AppState, app: &Router, raw: &str, full_placement: bool) -> String {
    let id = setup_event(state, app, raw, tournament(full_placement), 8, 2).await;
    let res = post(state, app, raw, &format!("/api/events/{id}/start"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    id
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
    res.body
}

fn nodes(d: &Value) -> &Vec<Value> {
    d["kind_data"]["nodes"].as_array().unwrap()
}

/// The bracket node labelled `label`.
fn node<'a>(d: &'a Value, label: &str) -> &'a Value {
    nodes(d)
        .iter()
        .find(|n| n["label"] == json!(label))
        .unwrap_or_else(|| panic!("no node {label}"))
}

fn match_id(d: &Value, label: &str) -> String {
    node(d, label)["match_id"].as_str().unwrap().to_owned()
}

fn match_in<'a>(d: &'a Value, match_id: &str) -> &'a Value {
    d["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(match_id))
        .unwrap()
}

fn scorer(d: &Value, match_id: &str) -> String {
    match_in(d, match_id)["scorer_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn quarter_final_ids(d: &Value) -> Vec<String> {
    (1..=4)
        .map(|i| match_id(d, &format!("Quarter-final {i}")))
        .collect()
}

static CLIENT: AtomicU64 = AtomicU64::new(0);

/// The coordinator types `[a-b]` for a match.
async fn typed(state: &AppState, app: &Router, raw: &str, id: &str, m: &str, a: u8, b: u8) {
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
    let res = post(
        state,
        app,
        raw,
        &format!("/api/events/{id}/matches/{m}/result"),
        Some(json!({
            "client_id": format!("typed-{n}"),
            "score": { "sets": [{ "a": a, "b": b, "tiebreak": null }] },
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// One point to side A through the scorer token.
async fn point(app: &Router, scorer_token: &str) {
    let n = CLIENT.fetch_add(1, Ordering::Relaxed);
    let res = send(
        app,
        req(
            Method::POST,
            &format!("/api/score/{scorer_token}/commands"),
            Some(json!({
                "client_id": format!("p-{n}"), "batch": false, "base_revision": 0,
                "commands": [{ "kind": "point", "team": "a" }],
            })),
            &[],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

/// Task 3's held bracket over the API: full placement, QF1 and QF2 won by side A, one point
/// on SF1, then QF1's winner flipped to side B.
async fn held(state: &AppState, app: &Router, raw: &str) -> String {
    let id = running(state, app, raw, true).await;
    let d = detail(state, app, raw, &id).await;
    let (q1, q2) = (
        match_id(&d, "Quarter-final 1"),
        match_id(&d, "Quarter-final 2"),
    );
    typed(state, app, raw, &id, &q1, 6, 0).await;
    typed(state, app, raw, &id, &q2, 6, 0).await;
    let d = detail(state, app, raw, &id).await;
    let sf1 = match_id(&d, "Semi-final 1");
    point(app, &scorer(&d, &sf1)).await;
    typed(state, app, raw, &id, &q1, 2, 6).await;
    id
}

fn decide_path(id: &str, match_id: &str) -> String {
    format!("/api/events/{id}/bracket/decisions/{match_id}")
}

fn route_paths(id: &str, match_id: &str) -> Vec<(String, Option<Value>)> {
    vec![
        (format!("/api/events/{id}/bracket/redraw"), None),
        (decide_path(id, match_id), Some(json!({ "choice": "keep" }))),
        (format!("/api/events/{id}/bracket/fill-courts"), None),
    ]
}

/// Every string anywhere in `v`.
fn strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(items) => items.iter().for_each(|i| strings(i, out)),
        Value::Object(map) => map.values().for_each(|i| strings(i, out)),
        _ => {}
    }
}

#[tokio::test]
async fn tournament_routes_need_session() {
    let (state, _pool, app, admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let m = match_id(&d, "Quarter-final 1");
    let token = device_token(&state.store, &admin).await;
    for (path, body) in route_paths(&id, &m) {
        let anon = send(&app, req(Method::POST, &path, body.clone(), &[])).await;
        assert_eq!(anon.status, StatusCode::UNAUTHORIZED, "{path}");
        let device = send(&app, as_device(&token, Method::POST, &path, body)).await;
        assert_eq!(device.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn other_users_tournament_is_404() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let m = match_id(&d, "Quarter-final 1");
    let (_b, braw) = invited_user(&state, &app, &raw, "b@x.test", "Rui").await;
    for (path, body) in route_paths(&id, &m) {
        let res = post(&state, &app, &braw, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    // A non-stop event of the owner is not a tournament.
    let non_stop = json!({
        "name": "Rally", "kind": "non_stop",
        "format": Format::Timed { deuce: DeuceRule::Advantage, draws_allowed: false },
    });
    let ns = setup_event(&state, &app, &raw, non_stop, 4, 1).await;
    let res = post(&state, &app, &raw, &format!("/api/events/{ns}/start"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    for (path, body) in route_paths(&ns, &m) {
        let res = post(&state, &app, &raw, &path, body).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    // An unknown match on the owner's tournament.
    let res = post(
        &state,
        &app,
        &raw,
        &decide_path(&id, "nope"),
        Some(json!({ "choice": "keep" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn detail_has_bracket() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = setup_event(&state, &app, &raw, tournament(true), 8, 2).await;
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(d["kind_data"], Value::Null, "no bracket in setup");
    let res = post(&state, &app, &raw, &format!("/api/events/{id}/start"), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let d = detail(&state, &app, &raw, &id).await;
    let k = &d["kind_data"];
    assert_eq!(k["full_placement"], json!(true));
    assert_eq!(k["teams"], json!(8));
    assert_eq!(k["can_redraw"], json!(true));
    assert_eq!(k["places"], Value::Null);
    let all = nodes(&d);
    assert_eq!(all.len(), 12);
    for n in all {
        assert!(n["label"].is_string(), "{n}");
        assert_eq!(n["from"].as_array().unwrap().len(), 2, "{n}");
        assert_eq!(n["teams"].as_array().unwrap().len(), 2, "{n}");
        assert_eq!(n["held"], json!(false), "{n}");
        assert!(n.get("replay_discards").is_none(), "{n}");
        let label = n["label"].as_str().unwrap();
        if label.starts_with("Quarter-final") {
            assert!(n["match_id"].is_string(), "{n}");
            assert_eq!(n["from"], json!([null, null]), "{n}");
            assert!(
                n["teams"][0].is_string() && n["teams"][1].is_string(),
                "{n}"
            );
        } else {
            assert_eq!(n["match_id"], Value::Null, "{n}");
            assert!(n["from"][0].is_object(), "{n}");
        }
    }
    assert_eq!(
        node(&d, "Semi-final 1")["from"],
        json!([
            { "winner_of": { "place_from": 1, "size": 8, "idx": 0 } },
            { "winner_of": { "place_from": 1, "size": 8, "idx": 1 } },
        ])
    );
    assert_eq!(
        node(&d, "Placement semi 1 (5th–8th)")["from"][1],
        json!({ "loser_of": { "place_from": 1, "size": 8, "idx": 1 } })
    );
    assert_eq!(nodes(&d)[0]["label"], json!("Quarter-final 1"));
}

#[tokio::test]
async fn spectator_bracket_has_no_tokens_or_coordinator_fields() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    assert!(node(&d, "Quarter-final 1")["replay_discards"].is_array());
    let mut secrets = Vec::new();
    for c in d["courts"].as_array().unwrap() {
        secrets.push(c["scorer_token"].as_str().unwrap().to_owned());
    }
    for m in d["matches"].as_array().unwrap() {
        secrets.push(m["scorer_token"].as_str().unwrap().to_owned());
    }
    let spectator = d["spectator_token"].as_str().unwrap();
    let res = send(
        &app,
        req(
            Method::GET,
            &format!("/api/event-view/{spectator}"),
            None,
            &[],
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let k = &res.body["kind_data"];
    assert_eq!(k["nodes"].as_array().unwrap().len(), 12);
    assert!(k.get("can_redraw").is_none(), "{k}");
    for n in k["nodes"].as_array().unwrap() {
        assert!(n.get("replay_discards").is_none(), "{n}");
    }
    assert_eq!(k["nodes"][0]["held"], json!(true));
    let mut found = Vec::new();
    strings(&res.body, &mut found);
    for secret in &secrets {
        assert!(
            !found.contains(&secret.as_str()),
            "token leaked: {}",
            res.body
        );
    }
}

#[tokio::test]
async fn court_state_has_no_bracket() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let court_token = d["courts"][0]["scorer_token"].as_str().unwrap();
    let res = send(
        &app,
        req(Method::GET, &format!("/api/court/{court_token}"), None, &[]),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["kind_data"], Value::Null);
}

#[tokio::test]
async fn redraw_until_first_point() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let old = quarter_final_ids(&d);
    let path = format!("/api/events/{id}/bracket/redraw");
    let res = post(&state, &app, &raw, &path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    let new = quarter_final_ids(&res.body);
    assert!(new.iter().all(|m| !old.contains(m)), "{old:?} {new:?}");
    assert_eq!(res.body["kind_data"]["can_redraw"], json!(true));
    // The old matches are gone, the new ones fill both courts.
    assert_eq!(res.body["matches"].as_array().unwrap().len(), 4);
    let on_courts: Vec<&Value> = res.body["courts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["current"])
        .collect();
    assert_eq!(on_courts, [&json!(new[0]), &json!(new[1])]);
    point(&app, &scorer(&res.body, &new[0])).await;
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(d["kind_data"]["can_redraw"], json!(false));
    let res = post(&state, &app, &raw, &path, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "play_started" }));
}

#[tokio::test]
async fn redraw_needs_running_event() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = setup_event(&state, &app, &raw, tournament(false), 4, 1).await;
    for path in [
        format!("/api/events/{id}/bracket/redraw"),
        format!("/api/events/{id}/bracket/fill-courts"),
    ] {
        let res = post(&state, &app, &raw, &path, None).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{path}");
        assert_eq!(res.body, json!({ "error": "event_not_running" }));
    }
}

#[tokio::test]
async fn redraw_closes_old_streams() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let old = scorer(&d, &match_id(&d, "Quarter-final 1"));
    let mut fence = Sse::open(
        &app,
        req(Method::GET, &format!("/api/score/{old}/stream"), None, &[]),
    )
    .await;
    fence.next("state").await;
    let res = post(
        &state,
        &app,
        &raw,
        &format!("/api/events/{id}/bracket/redraw"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(fence.ends_within(Duration::from_secs(2)).await);
}

#[tokio::test]
async fn held_detail_lists_discards() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let q1 = node(&d, "Quarter-final 1");
    assert_eq!(q1["held"], json!(true));
    assert_eq!(
        q1["replay_discards"],
        json!(["Semi-final 1", "Placement semi 1 (5th–8th)"])
    );
    assert!(node(&d, "Quarter-final 2").get("replay_discards").is_none());
    assert_eq!(d["kind_data"]["can_redraw"], json!(false));
}

#[tokio::test]
async fn decision_keep_clears_hold() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let (q1, sf1) = (
        match_id(&d, "Quarter-final 1"),
        match_id(&d, "Semi-final 1"),
    );
    let res = post(
        &state,
        &app,
        &raw,
        &decide_path(&id, &q1),
        Some(json!({ "choice": "keep" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(node(&res.body, "Quarter-final 1")["held"], json!(false));
    assert_eq!(match_id(&res.body, "Semi-final 1"), sf1, "kept as played");
}

#[tokio::test]
async fn decision_without_hold_is_409() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let q2 = match_id(&d, "Quarter-final 2");
    for choice in ["keep", "replay"] {
        let res = post(
            &state,
            &app,
            &raw,
            &decide_path(&id, &q2),
            Some(json!({ "choice": choice })),
        )
        .await;
        assert_eq!(res.status, StatusCode::CONFLICT);
        assert_eq!(res.body, json!({ "error": "no_decision" }));
    }
}

#[tokio::test]
async fn decision_bad_choice_is_422() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let q1 = match_id(&d, "Quarter-final 1");
    for body in [
        json!({ "choice": "maybe" }),
        json!({}),
        json!({ "choice": 1 }),
    ] {
        let res = post(&state, &app, &raw, &decide_path(&id, &q1), Some(body)).await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(
            res.body,
            json!({ "error": "validation", "field": "choice" })
        );
    }
    let d = detail(&state, &app, &raw, &id).await;
    assert_eq!(node(&d, "Quarter-final 1")["held"], json!(true));
}

#[tokio::test]
async fn decision_replay_closes_old_streams() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = held(&state, &app, &raw).await;
    let d = detail(&state, &app, &raw, &id).await;
    let q1 = match_id(&d, "Quarter-final 1");
    let corrected_winner = node(&d, "Quarter-final 1")["teams"][1].clone();
    let old_sf1 = match_id(&d, "Semi-final 1");
    let old_token = scorer(&d, &old_sf1);
    let mut fence = Sse::open(
        &app,
        req(
            Method::GET,
            &format!("/api/score/{old_token}/stream"),
            None,
            &[],
        ),
    )
    .await;
    fence.next("state").await;
    let mut events = Sse::open(
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
    events.next("event").await;
    let res = post(
        &state,
        &app,
        &raw,
        &decide_path(&id, &q1),
        Some(json!({ "choice": "replay" })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let sf1 = node(&res.body, "Semi-final 1");
    assert_ne!(sf1["match_id"], json!(old_sf1));
    assert!(sf1["match_id"].is_string());
    assert_eq!(sf1["teams"][0], corrected_winner);
    assert_eq!(node(&res.body, "Quarter-final 1")["held"], json!(false));
    assert!(fence.ends_within(Duration::from_secs(2)).await);
    let frame = events.next("event").await;
    assert_eq!(node(&frame, "Quarter-final 1")["held"], json!(false));
}

#[tokio::test]
async fn fill_courts_after_court_added() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = running(&state, &app, &raw, false).await;
    let d = detail(&state, &app, &raw, &id).await;
    let q3 = match_id(&d, "Quarter-final 3");
    assert_eq!(match_in(&d, &q3)["court_id"], Value::Null);
    let third = add_court(&state, &app, &raw, &id).await;
    let res = post(
        &state,
        &app,
        &raw,
        &format!("/api/events/{id}/bracket/fill-courts"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(match_in(&res.body, &q3)["court_id"], json!(third));
    let q4 = match_id(&res.body, "Quarter-final 4");
    assert_eq!(match_in(&res.body, &q4)["court_id"], Value::Null);
}

#[tokio::test]
async fn start_error_is_team_count() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let id = setup_event(&state, &app, &raw, tournament(false), 6, 1).await;
    let res = post(&state, &app, &raw, &format!("/api/events/{id}/start"), None).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "team_count" }));
}
