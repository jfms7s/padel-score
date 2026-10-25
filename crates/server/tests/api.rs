#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use padel_score_server::{AppState, Config, Store, db, router};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use tower::ServiceExt;

const TIMEOUT: Duration = Duration::from_secs(2);

async fn setup() -> (Router, AppState) {
    let store = Store::new(db::connect("sqlite::memory:").await.unwrap());
    let state = AppState::new(Arc::new(Config::from_map(&HashMap::new()).unwrap()), store);
    (router(state.clone()), state)
}

fn new_match_body() -> Value {
    json!({
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
    })
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

async fn get(app: &Router, path: &str) -> (StatusCode, HeaderMap, Value) {
    send(app, Request::get(path).body(Body::empty()).unwrap()).await
}

async fn post(app: &Router, path: &str, body: &Value) -> (StatusCode, HeaderMap, Value) {
    post_raw(app, path, body.to_string()).await
}

async fn post_raw(app: &Router, path: &str, body: String) -> (StatusCode, HeaderMap, Value) {
    send(
        app,
        Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

struct Created {
    id: String,
    scorer: String,
    spectator: String,
}

async fn create(app: &Router) -> Created {
    let (status, _, body) = post(app, "/api/matches", &new_match_body()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    Created {
        id: body["id"].as_str().unwrap().to_owned(),
        scorer: body["scorer_token"].as_str().unwrap().to_owned(),
        spectator: body["spectator_token"].as_str().unwrap().to_owned(),
    }
}

fn live(client_id: &str, commands: Value) -> Value {
    json!({ "client_id": client_id, "batch": false, "base_revision": 0, "commands": commands })
}

async fn command(app: &Router, scorer: &str, body: &Value) -> (StatusCode, Value) {
    let (status, _, body) = post(app, &format!("/api/score/{scorer}/commands"), body).await;
    (status, body)
}

/// An open SSE response body, read one `state` event at a time.
struct Stream {
    body: Body,
    buf: String,
}

impl Stream {
    async fn open(app: &Router, path: &str) -> Stream {
        let res = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        Stream {
            body: res.into_body(),
            buf: String::new(),
        }
    }

    /// The next frame's bytes, or `None` once the body has ended.
    async fn frame(&mut self) -> Option<String> {
        loop {
            let frame = tokio::time::timeout(TIMEOUT, self.body.frame())
                .await
                .expect("timed out waiting for a frame")?
                .unwrap();
            if let Ok(data) = frame.into_data() {
                return Some(String::from_utf8(data.to_vec()).unwrap());
            }
        }
    }

    /// The next `state` event's data, skipping keep-alives.
    async fn state(&mut self) -> Value {
        loop {
            if let Some(end) = self.buf.find("\n\n") {
                let event: String = self.buf.drain(..end + 2).collect();
                let mut name = None;
                let mut data = String::new();
                for line in event.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        name = Some(v.trim().to_owned());
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.trim_start());
                    }
                }
                if name.as_deref() == Some("state") {
                    return serde_json::from_str(&data).unwrap();
                }
                continue;
            }
            let chunk = self.frame().await.expect("stream ended");
            self.buf.push_str(&chunk);
        }
    }
}

#[tokio::test]
async fn create_match_returns_tokens() {
    let (app, _) = setup().await;
    let (status, headers, body) = post(&app, "/api/matches", &new_match_body()).await;
    assert_eq!(status, StatusCode::CREATED);
    let scorer = body["scorer_token"].as_str().unwrap();
    let spectator = body["spectator_token"].as_str().unwrap();
    assert_eq!(scorer.len(), 43);
    assert_eq!(spectator.len(), 43);
    assert_ne!(scorer, spectator);
    assert!(uuid::Uuid::parse_str(body["id"].as_str().unwrap()).is_ok());
    assert_eq!(headers[header::LOCATION], format!("/s/{scorer}"));
    assert_eq!(body.as_object().unwrap().len(), 3);
}

#[tokio::test]
async fn invalid_match_is_422() {
    let (app, _) = setup().await;
    let mut body = new_match_body();
    body["teams"]["a"][0] = json!("x".repeat(41));
    let (status, _, body) = post(&app, "/api/matches", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "invalid_match");
    assert!(body["message"].is_string());

    let mut body = new_match_body();
    body["start"]["other_first_server"] = json!(7);
    let (status, _, body) = post(&app, "/api/matches", &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "invalid_match");
}

#[tokio::test]
async fn malformed_json_is_400() {
    let (app, _) = setup().await;
    let (status, headers, body) = post_raw(&app, "/api/matches", "{".into()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("json")
    );
    assert_eq!(body, json!({ "error": "bad_request" }));

    // Well-formed JSON of the wrong shape is a bad request too.
    let (status, _, body) = post(&app, "/api/matches", &json!({ "format": 1 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({ "error": "bad_request" }));

    let m = create(&app).await;
    let (status, body) = command(&app, &m.scorer, &json!({ "commands": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({ "error": "bad_request" }));
}

#[tokio::test]
async fn oversized_body_is_413() {
    let (app, _) = setup().await;
    let mut body = new_match_body();
    body["padding"] = json!("x".repeat(65 * 1024));
    let (status, _, _) = post(&app, "/api/matches", &body).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn scorer_and_spectator_can_read() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    let (status, _, by_scorer) = get(&app, &format!("/api/score/{}", m.scorer)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, by_spectator) = get(&app, &format!("/api/view/{}", m.spectator)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, view_by_scorer) = get(&app, &format!("/api/view/{}", m.scorer)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(by_scorer, by_spectator);
    assert_eq!(by_scorer, view_by_scorer);
    assert_eq!(by_scorer["id"], m.id.as_str());
    assert_eq!(by_scorer["revision"], 0);
    assert_eq!(by_scorer["head_seq"], 0);
    let text = by_scorer.to_string();
    assert!(!text.contains(&m.scorer) && !text.contains(&m.spectator));
}

#[tokio::test]
async fn unknown_or_wrong_token_is_404() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    for path in [
        format!("/api/score/{}", m.spectator),
        "/api/score/nope".to_owned(),
        "/api/view/nope".to_owned(),
        format!("/api/score/{}/stream", m.spectator),
        "/api/view/nope/stream".to_owned(),
    ] {
        let (status, _, body) = get(&app, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body, json!({ "error": "not_found" }), "{path}");
    }
}

#[tokio::test]
async fn spectator_token_cannot_post_commands() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    let (status, body) = command(
        &app,
        &m.spectator,
        &live("c1", json!([{"kind":"point","team":"a"}])),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "not_found" }));
    let replace = json!({ "client_id": "r1", "base_revision": 0, "base_seq": 0, "commands": [{"kind":"point","team":"a"}] });
    let (status, _, body) = post(
        &app,
        &format!("/api/score/{}/replace", m.spectator),
        &replace,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "not_found" }));
    let (_, _, view) = get(&app, &format!("/api/view/{}", m.spectator)).await;
    assert_eq!(view["head_seq"], 0);
}

#[tokio::test]
async fn command_applies_and_returns_view() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    let (status, body) = command(
        &app,
        &m.scorer,
        &live("c1", json!([{"kind":"point","team":"a"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["revision"], 1);
    assert_eq!(body["head_seq"], 1);
    assert_eq!(
        body["events"],
        json!([{ "seq": 1, "event": { "point": "a" } }])
    );
}

#[tokio::test]
async fn command_conflict_is_409_with_view() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    let (status, _) = command(
        &app,
        &m.scorer,
        &live("c1", json!([{"kind":"point","team":"a"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stale = json!({ "client_id": "b1", "batch": true, "base_revision": 0, "commands": [{"kind":"point","team":"b"}] });
    let (status, body) = command(&app, &m.scorer, &stale).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    assert_eq!(body["view"]["revision"], 1);
    assert_eq!(body["view"]["head_seq"], 1);
}

#[tokio::test]
async fn rejected_command_is_422_with_kind() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    let (status, body) = command(&app, &m.scorer, &live("c1", json!([{"kind":"time_up"}]))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body,
        json!({ "error": "rejected", "kind": "not_timed", "index": 0 })
    );
}

#[tokio::test]
async fn replayed_resend_republishes_current_view() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let req = live("c1", json!([{"kind":"point","team":"a"}]));
    let (_, first) = command(&app, &m.scorer, &req).await;
    let (status, _) = command(
        &app,
        &m.scorer,
        &live("c2", json!([{"kind":"point","team":"b"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut rx = state.hub.subscribe(&m.id);
    let (status, again) = command(&app, &m.scorer, &req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first, again, "the resend gets the stored body");
    // In case the first request's publish was lost, the streams get the current view.
    let published = rx.try_recv().expect("a 200 replay republishes");
    assert_eq!(published.revision, 2);
    assert_eq!(published.head_seq, 2);
}

#[tokio::test]
async fn replayed_rejection_is_not_published() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let req = live("c1", json!([{"kind":"time_up"}]));
    let (status, _) = command(&app, &m.scorer, &req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let mut rx = state.hub.subscribe(&m.id);
    let (status, _) = command(&app, &m.scorer, &req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        rx.try_recv().is_err(),
        "a replayed rejection changed nothing"
    );
}

#[tokio::test]
async fn wrong_method_is_json_405() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    for (method, path) in [
        ("GET", format!("/api/score/{}/commands", m.scorer)),
        ("POST", format!("/api/score/{}", m.scorer)),
        ("DELETE", "/api/matches".to_owned()),
        ("POST", format!("/api/view/{}/stream", m.spectator)),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(&path)
            .body(Body::empty())
            .unwrap();
        let (status, headers, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        assert!(
            headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("json")
        );
        assert_eq!(
            body,
            json!({ "error": "method_not_allowed" }),
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn bad_path_encoding_is_json_400() {
    let (app, _) = setup().await;
    for path in ["/api/score/%FF", "/api/view/%C3%28/stream"] {
        let (status, _, body) = get(&app, path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(body, json!({ "error": "bad_request" }), "{path}");
    }
    let (status, _, body) = post(
        &app,
        "/api/score/%FF/commands",
        &live("c1", json!([{"kind":"time_up"}])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({ "error": "bad_request" }));
}

#[tokio::test]
async fn replace_applies_and_publishes() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let two = live(
        "c1",
        json!([{"kind":"point","team":"a"},{"kind":"point","team":"a"}]),
    );
    let (status, _) = command(&app, &m.scorer, &two).await;
    assert_eq!(status, StatusCode::OK);
    let mut rx = state.hub.subscribe(&m.id);
    let replace = json!({ "client_id": "r1", "base_revision": 1, "base_seq": 1, "commands": [{"kind":"point","team":"b"}] });
    let (status, _, body) = post(&app, &format!("/api/score/{}/replace", m.scorer), &replace).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["revision"], 2);
    assert_eq!(body["head_seq"], 3);
    let seqs: Vec<i64> = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![1, 3]);
    let published = rx.try_recv().unwrap();
    assert_eq!(published.revision, 2);

    // A stale replace is a conflict and publishes nothing.
    let stale = json!({ "client_id": "r2", "base_revision": 1, "base_seq": 0, "commands": [{"kind":"point","team":"b"}] });
    let (status, _, body) = post(&app, &format!("/api/score/{}/replace", m.scorer), &stale).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["view"]["revision"], 2);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn stream_sends_state_first_and_after_command() {
    let (app, _) = setup().await;
    let m = create(&app).await;
    for path in [
        format!("/api/score/{}/stream", m.scorer),
        format!("/api/view/{}/stream", m.spectator),
        format!("/api/view/{}/stream", m.scorer),
    ] {
        let mut stream = Stream::open(&app, &path).await;
        let first = stream.state().await;
        let (_, _, current) = get(&app, &format!("/api/view/{}", m.spectator)).await;
        assert_eq!(first, current, "{path}");
        let head = first["head_seq"].as_i64().unwrap();
        let client_id = format!("c-{head}");
        let (status, applied) = command(
            &app,
            &m.scorer,
            &live(&client_id, json!([{"kind":"point","team":"a"}])),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let next = stream.state().await;
        assert_eq!(next["head_seq"].as_i64().unwrap(), head + 1, "{path}");
        assert_eq!(next, applied, "{path}");
        let text = next.to_string();
        assert!(!text.contains(&m.scorer) && !text.contains(&m.spectator));
    }
}

#[tokio::test]
async fn lagged_stream_rereads_view() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let mut stream = Stream::open(&app, &format!("/api/view/{}/stream", m.spectator)).await;
    let first = stream.state().await;
    // The fakes below never reach the store: they only fill the channel past its capacity so
    // the receiver lags. Their revisions (100+) are above anything stored, so if the stream
    // skipped the lag silently, the next frame would be a fake rather than the stored view.
    let view: padel_score_server::commands::MatchView =
        serde_json::from_value(first.clone()).unwrap();
    // Overflow the channel (capacity 16) with fake views the store never had.
    for i in 0..20 {
        let mut fake = view.clone();
        fake.revision = 100 + i;
        state.hub.publish(fake);
    }
    let after_lag = stream.state().await;
    assert_eq!(after_lag, first, "a lagged stream re-reads the stored view");
}

#[tokio::test]
async fn streams_end_on_shutdown() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let mut stream = Stream::open(&app, &format!("/api/score/{}/stream", m.scorer)).await;
    stream.state().await;
    state.shutdown.cancel();
    while stream.frame().await.is_some() {}
}

#[tokio::test]
async fn hub_drops_entry_when_last_stream_goes() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let mut a = Stream::open(&app, &format!("/api/view/{}/stream", m.spectator)).await;
    let mut b = Stream::open(&app, &format!("/api/score/{}/stream", m.scorer)).await;
    a.state().await;
    b.state().await;
    assert_eq!(state.hub.len(), 1);
    drop(a);
    assert_eq!(state.hub.len(), 1);
    drop(b);
    assert!(state.hub.is_empty());

    // Publishing to a match nobody watches creates nothing.
    let (status, _) = command(
        &app,
        &m.scorer,
        &live("c1", json!([{"kind":"point","team":"a"}])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(state.hub.is_empty());
}

#[tokio::test]
async fn closing_a_hub_entry_ends_its_stream() {
    let (app, state) = setup().await;
    let m = create(&app).await;
    let mut stream = Stream::open(&app, &format!("/api/view/{}/stream", m.spectator)).await;
    stream.state().await;
    assert!(state.hub.is_watched(&m.id));
    state.hub.close(&m.id);
    assert!(!state.hub.is_watched(&m.id));
    while stream.frame().await.is_some() {}
}
