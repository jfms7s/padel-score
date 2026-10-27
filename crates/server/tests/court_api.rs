//! The court phone's routes (court state, its stream with the phone count, advance) and the
//! event spectator routes (phase 6, Task 7).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::time::Duration;

use axum::Router;
use axum::http::{Method, StatusCode, header};
use padel_score_server::AppState;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use support::{Res, Sse, as_session, req, send, with_admin};

fn one_set() -> Value {
    serde_json::to_value(Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    }))
    .unwrap()
}

/// A non-stop event (timed matches, places worked out from the standings at Finish).
fn non_stop() -> Value {
    json!({
        "name": "Sunday cup",
        "kind": "non_stop",
        "format": Format::Timed { deuce: DeuceRule::Advantage, draws_allowed: false },
    })
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

/// A request with no credential at all, as a court phone or spectator sends it.
async fn anon(app: &Router, method: Method, path: &str, body: Option<Value>) -> Res {
    send(app, req(method, path, body, &[])).await
}

async fn ok(res: Res) -> Value {
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body
}

struct Event {
    id: String,
    teams: Vec<String>,
    courts: Vec<String>,
    /// The match created current on court 1.
    first: String,
}

/// Starting a tournament draws its bracket and puts matches on the courts. These tests are about
/// courts and hand-made event matches, so remove the drawn matches (their courts go free).
async fn clear_draw(pool: &sqlx::SqlitePool) {
    sqlx::query("DELETE FROM matches WHERE id IN (SELECT match_id FROM bracket_matches)")
        .execute(pool)
        .await
        .unwrap();
}

/// A running "Sunday cup" with four teams, two courts and one match current on court 1.
async fn running_event(
    state: &AppState,
    pool: &sqlx::SqlitePool,
    app: &Router,
    raw: &str,
) -> Event {
    let tournament = json!({ "name": "Sunday cup", "kind": "tournament", "format": one_set() });
    running_event_of(state, pool, app, raw, tournament).await
}

/// [`running_event`] for the event `new` (the body of `POST /api/events`).
async fn running_event_of(
    state: &AppState,
    pool: &sqlx::SqlitePool,
    app: &Router,
    raw: &str,
    new: Value,
) -> Event {
    let res = call(state, app, raw, Method::POST, "/api/events", Some(new)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    let mut teams = Vec::new();
    for [p1, p2] in [
        ["Ana", "Marta"],
        ["Rui", "João"],
        ["Eva", "Inês"],
        ["Tiago", "Luís"],
    ] {
        let mut players = Vec::new();
        for name in [p1, p2] {
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
            players.push(res.body["id"].as_str().unwrap().to_owned());
        }
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
        courts.push(res.body["id"].as_str().unwrap().to_owned());
    }
    ok(call(
        state,
        app,
        raw,
        Method::POST,
        &format!("/api/events/{id}/start"),
        None,
    )
    .await)
    .await;
    clear_draw(pool).await;
    let first = create_match(state, app, raw, &id, &teams[0], &teams[1], &courts[0]).await;
    Event {
        id,
        teams,
        courts,
        first,
    }
}

async fn create_match(
    state: &AppState,
    app: &Router,
    raw: &str,
    id: &str,
    a: &str,
    b: &str,
    court: &str,
) -> String {
    let res = call(
        state,
        app,
        raw,
        Method::POST,
        &format!("/api/events/{id}/matches"),
        Some(json!({ "team_a": a, "team_b": b, "court_id": court })),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

async fn detail(state: &AppState, app: &Router, raw: &str, id: &str) -> Value {
    ok(call(
        state,
        app,
        raw,
        Method::GET,
        &format!("/api/events/{id}"),
        None,
    )
    .await)
    .await
}

fn court_in<'a>(detail: &'a Value, court_id: &str) -> &'a Value {
    detail["courts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(court_id))
        .unwrap()
}

fn match_in<'a>(detail: &'a Value, match_id: &str) -> &'a Value {
    detail["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(match_id))
        .unwrap()
}

/// The court's (scorer token, spectator token).
fn court_tokens(detail: &Value, court_id: &str) -> (String, String) {
    let c = court_in(detail, court_id);
    (
        c["scorer_token"].as_str().unwrap().to_owned(),
        c["spectator_token"].as_str().unwrap().to_owned(),
    )
}

fn assert_secret(res: &Res) {
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    assert_eq!(res.header(header::REFERRER_POLICY), Some("no-referrer"));
}

/// Finish a match by typing 6–2 through its scorer token.
async fn finish_match(app: &Router, scorer_token: &str) {
    let res = anon(
        app,
        Method::POST,
        &format!("/api/score/{scorer_token}/final"),
        Some(json!({
            "client_id": "f1", "base_revision": 0,
            "score": { "sets": [{ "a": 6, "b": 2, "tiebreak": null }] },
        })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

async fn open(app: &Router, path: &str) -> Sse {
    Sse::open(app, req(Method::GET, path, None, &[])).await
}

#[tokio::test]
async fn court_state_scorer_gets_scorer_token_viewer_does_not() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let second = create_match(
        &state,
        &app,
        &raw,
        &ev.id,
        &ev.teams[2],
        &ev.teams[3],
        &ev.courts[0],
    )
    .await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, viewer) = court_tokens(&d, &ev.courts[0]);
    let first = match_in(&d, &ev.first);

    let res = anon(&app, Method::GET, &format!("/api/court/{scorer}"), None).await;
    assert_secret(&res);
    let body = ok(res).await;
    assert_eq!(body["event"]["id"], json!(ev.id));
    assert_eq!(body["event"]["name"], json!("Sunday cup"));
    assert_eq!(body["event"]["kind"], json!("tournament"));
    assert_eq!(body["event"]["status"], json!("running"));
    assert_eq!(body["court"]["id"], json!(ev.courts[0]));
    assert_eq!(body["court"]["name"], json!("Court 1"));
    assert_eq!(body["current"]["id"], json!(ev.first));
    assert_eq!(body["current"]["label"], json!("Match 1"));
    assert_eq!(body["current"]["teams"]["a"], json!(["Ana", "Marta"]));
    assert_eq!(body["current"]["spectator_token"], first["spectator_token"]);
    assert_eq!(body["current"]["scorer_token"], first["scorer_token"]);
    assert_eq!(body["next"]["id"], json!(second));
    assert_eq!(body["next"]["label"], json!("Match 2"));
    assert_eq!(body["kind_data"], Value::Null);

    let res = anon(&app, Method::GET, &format!("/api/court/{viewer}"), None).await;
    assert_secret(&res);
    let body = ok(res).await;
    assert_eq!(body["current"]["id"], json!(ev.first));
    assert_eq!(body["current"]["spectator_token"], first["spectator_token"]);
    assert!(body["current"].get("scorer_token").is_none(), "{body}");
    assert!(body["next"].get("scorer_token").is_none(), "{body}");

    let unknown = anon(&app, Method::GET, "/api/court/nope", None).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body, json!({ "error": "not_found" }));
}

#[tokio::test]
async fn phones_count_follows_scorer_streams() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, viewer) = court_tokens(&d, &ev.courts[0]);
    assert_eq!(court_in(&d, &ev.courts[0])["phones"], json!(0));

    let mut phone = open(&app, &format!("/api/court/{scorer}/stream")).await;
    phone.next("court").await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    assert_eq!(court_in(&d, &ev.courts[0])["phones"], json!(1));
    assert_eq!(court_in(&d, &ev.courts[1])["phones"], json!(0));

    let mut watcher = open(&app, &format!("/api/court/{viewer}/stream")).await;
    watcher.next("court").await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    assert_eq!(court_in(&d, &ev.courts[0])["phones"], json!(1));

    drop(phone);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let d = detail(&state, &app, &raw, &ev.id).await;
        if court_in(&d, &ev.courts[0])["phones"] == json!(0) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "phone still counted"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(watcher);
}

#[tokio::test]
async fn court_stream_follows_advance() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let second = create_match(
        &state,
        &app,
        &raw,
        &ev.id,
        &ev.teams[2],
        &ev.teams[3],
        &ev.courts[0],
    )
    .await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, _) = court_tokens(&d, &ev.courts[0]);

    let mut phone = open(&app, &format!("/api/court/{scorer}/stream")).await;
    let first = phone.next("court").await;
    assert_eq!(first["current"]["id"], json!(ev.first));
    assert_eq!(first["next"]["id"], json!(second));
    let first_token = first["current"]["scorer_token"]
        .as_str()
        .unwrap()
        .to_owned();

    finish_match(&app, &first_token).await;
    let frame = phone.next("court").await;
    // The finished match stays on the court, so its result shows.
    assert_eq!(frame["current"]["id"], json!(ev.first));
    assert_eq!(frame["next"]["id"], json!(second));

    let res = anon(
        &app,
        Method::POST,
        &format!("/api/court/{scorer}/advance"),
        None,
    )
    .await;
    assert_secret(&res);
    let body = ok(res).await;
    assert_eq!(body["current"]["id"], json!(second));
    assert_eq!(body["next"], Value::Null);

    let frame = phone.next("court").await;
    assert_eq!(frame["current"]["id"], json!(second));
    let token = frame["current"]["scorer_token"].as_str().unwrap();
    assert_ne!(token, first_token);
    assert_eq!(frame["next"], Value::Null);
    // The match that left the court has a new scorer token: the old link is dead.
    let old = anon(
        &app,
        Method::GET,
        &format!("/api/score/{first_token}"),
        None,
    )
    .await;
    assert_eq!(old.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn advance_by_viewer_is_403() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, viewer) = court_tokens(&d, &ev.courts[0]);
    finish_match(
        &app,
        match_in(&d, &ev.first)["scorer_token"].as_str().unwrap(),
    )
    .await;
    let res = anon(
        &app,
        Method::POST,
        &format!("/api/court/{viewer}/advance"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body, json!({ "error": "forbidden" }));
    // Nothing moved.
    let body = ok(anon(&app, Method::GET, &format!("/api/court/{scorer}"), None).await).await;
    assert_eq!(body["current"]["id"], json!(ev.first));
    let unknown = anon(&app, Method::POST, "/api/court/nope/advance", None).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn advance_unfinished_is_409() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, _) = court_tokens(&d, &ev.courts[0]);
    let res = anon(
        &app,
        Method::POST,
        &format!("/api/court/{scorer}/advance"),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "match_not_finished" }));
}

#[tokio::test]
async fn rotated_court_token_is_404_and_stream_ends() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, viewer) = court_tokens(&d, &ev.courts[0]);
    let mut phone = open(&app, &format!("/api/court/{scorer}/stream")).await;
    phone.next("court").await;

    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{}/courts/{}/rotate", ev.id, ev.courts[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(phone.ends_within(Duration::from_secs(2)).await);

    for (method, path) in [
        (Method::GET, format!("/api/court/{scorer}")),
        (Method::GET, format!("/api/court/{scorer}/stream")),
        (Method::POST, format!("/api/court/{scorer}/advance")),
    ] {
        let res = anon(&app, method.clone(), &path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{method} {path}");
    }
    // The spectator token stays.
    ok(anon(&app, Method::GET, &format!("/api/court/{viewer}"), None).await).await;
    // The phone is no longer counted.
    let d = detail(&state, &app, &raw, &ev.id).await;
    assert_eq!(court_in(&d, &ev.courts[0])["phones"], json!(0));
}

#[tokio::test]
async fn finished_event_court_scorer_is_410_viewer_ok() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    // Phase 6 finished this with typed places; a non-stop works its own out, so Finish sends
    // none. What this test is about is the court links once the event is finished.
    let ev = running_event_of(&state, &pool, &app, &raw, non_stop()).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, viewer) = court_tokens(&d, &ev.courts[0]);
    let mut phone = open(&app, &format!("/api/court/{scorer}/stream")).await;
    phone.next("court").await;
    let mut watcher = open(&app, &format!("/api/court/{viewer}/stream")).await;
    watcher.next("court").await;

    let res = call(
        &state,
        &app,
        &raw,
        Method::POST,
        &format!("/api/events/{}/finish", ev.id),
        Some(json!({})),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(phone.ends_within(Duration::from_secs(2)).await);
    // The viewer's stream hears that the event finished and stays open.
    let frame = watcher.next("court").await;
    assert_eq!(frame["event"]["status"], json!("finished"));

    for (method, path) in [
        (Method::GET, format!("/api/court/{scorer}")),
        (Method::GET, format!("/api/court/{scorer}/stream")),
        (Method::POST, format!("/api/court/{scorer}/advance")),
    ] {
        let res = anon(&app, method.clone(), &path, None).await;
        assert_eq!(res.status, StatusCode::GONE, "{method} {path}");
        assert_eq!(res.body, json!({ "error": "event_finished" }));
    }
    let body = ok(anon(&app, Method::GET, &format!("/api/court/{viewer}"), None).await).await;
    assert_eq!(body["event"]["status"], json!("finished"));
    assert_eq!(body["current"], Value::Null);
    let mut late = open(&app, &format!("/api/court/{viewer}/stream")).await;
    assert_eq!(
        late.next("court").await["event"]["status"],
        json!("finished")
    );
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
async fn event_view_has_no_scorer_tokens() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    create_match(
        &state,
        &app,
        &raw,
        &ev.id,
        &ev.teams[2],
        &ev.teams[3],
        &ev.courts[1],
    )
    .await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let spectator = d["spectator_token"].as_str().unwrap();
    let mut secrets: Vec<String> = Vec::new();
    for c in d["courts"].as_array().unwrap() {
        secrets.push(c["scorer_token"].as_str().unwrap().to_owned());
    }
    for m in d["matches"].as_array().unwrap() {
        secrets.push(m["scorer_token"].as_str().unwrap().to_owned());
    }
    assert_eq!(secrets.len(), 4);

    let res = anon(
        &app,
        Method::GET,
        &format!("/api/event-view/{spectator}"),
        None,
    )
    .await;
    assert_secret(&res);
    let view = ok(res).await;
    let mut found = Vec::new();
    strings(&view, &mut found);
    for secret in &secrets {
        assert!(
            !found.contains(&secret.as_str()),
            "scorer token leaked: {view}"
        );
    }
    assert_eq!(view["event"]["name"], json!("Sunday cup"));
    assert_eq!(view["teams"].as_array().unwrap().len(), 4);
    assert_eq!(view["matches"].as_array().unwrap().len(), 2);
    let court = &view["courts"][0];
    assert!(court.get("phones").is_none(), "{court}");
    assert!(court.get("scorer_token").is_none(), "{court}");
    assert_eq!(
        court["spectator_token"],
        court_in(&d, &ev.courts[0])["spectator_token"]
    );
    assert_eq!(court["current"], json!(ev.first));
    let m = match_in(&view, &ev.first);
    assert!(m.get("scorer_token").is_none(), "{m}");
    assert_eq!(
        m["spectator_token"],
        match_in(&d, &ev.first)["spectator_token"]
    );

    let mut stream = open(&app, &format!("/api/event-view/{spectator}/stream")).await;
    let frame = stream.next("event").await;
    assert_eq!(frame, view);
    // A change reaches it, still without scorer tokens.
    finish_match(&app, &secrets[2]).await;
    let frame = stream.next("event").await;
    let mut found = Vec::new();
    strings(&frame, &mut found);
    for secret in &secrets {
        assert!(
            !found.contains(&secret.as_str()),
            "scorer token leaked: {frame}"
        );
    }
    assert_eq!(match_in(&frame, &ev.first)["state"], json!("finished"));

    for path in ["/api/event-view/nope", "/api/event-view/nope/stream"] {
        let res = anon(&app, Method::GET, path, None).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    // A court's or a match's token is not an event spectator token.
    let res = anon(
        &app,
        Method::GET,
        &format!("/api/event-view/{}", secrets[0]),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn assign_reaches_waiting_court_phone() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &raw).await;
    let d = detail(&state, &app, &raw, &ev.id).await;
    let (scorer, _) = court_tokens(&d, &ev.courts[1]);
    let mut phone = open(&app, &format!("/api/court/{scorer}/stream")).await;
    let first = phone.next("court").await;
    assert_eq!(first["current"], Value::Null);
    assert_eq!(first["next"], Value::Null);

    let m = create_match(
        &state,
        &app,
        &raw,
        &ev.id,
        &ev.teams[2],
        &ev.teams[3],
        &ev.courts[1],
    )
    .await;
    let frame = phone.next("court").await;
    assert_eq!(frame["current"]["id"], json!(m));
    assert!(frame["current"]["scorer_token"].is_string());
}

#[tokio::test]
async fn event_match_cannot_be_deleted_by_its_scorer_token() {
    let (state, pool, app, _, admin) = with_admin().await;
    let ev = running_event(&state, &pool, &app, &admin).await;
    let scorer: String = sqlx::query_scalar("SELECT scorer_token FROM matches WHERE id = ?")
        .bind(&ev.first)
        .fetch_one(&pool)
        .await
        .unwrap();
    let path = format!("/api/score/{scorer}");
    let res = anon(&app, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body, json!({ "error": "event_match" }));
    assert_eq!(
        anon(&app, Method::GET, &path, None).await.status,
        StatusCode::OK
    );
}
