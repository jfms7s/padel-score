#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Web push: the VAPID key, device subscriptions through the API, and the outbox sender against
//! a fake transport.

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::future::BoxFuture;
use padel_score_server::push::{
    self, MAX_ATTEMPTS, Notice, NoticeKind, Payload, PushKeys, PushSubscription, PushTransport,
    Pusher, SendReport, send_pending, vapid_keys,
};
use padel_score_server::{AppState, Store};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{as_device, as_session, device_token, invited_user, req, send, with_admin};
use web_push_native::jwt_simple::algorithms::ES256KeyPair;

/// Records every request and answers from a queue of statuses (201 once it is empty).
#[derive(Default)]
struct FakeTransport {
    requests: Mutex<Vec<http::Request<Vec<u8>>>>,
    statuses: Mutex<VecDeque<u16>>,
}

impl FakeTransport {
    fn answering(statuses: &[u16]) -> Arc<FakeTransport> {
        Arc::new(FakeTransport {
            requests: Mutex::default(),
            statuses: Mutex::new(statuses.iter().copied().collect()),
        })
    }
}

impl PushTransport for FakeTransport {
    fn send(&self, req: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<u16, String>> {
        self.requests.lock().unwrap().push(req);
        let status = self.statuses.lock().unwrap().pop_front().unwrap_or(201);
        Box::pin(async move { Ok(status) })
    }
}

fn with_fake(state: &mut AppState, fake: &Arc<FakeTransport>) {
    let transport: Arc<dyn PushTransport> = fake.clone();
    state.push = Arc::new(Pusher::new(transport));
}

/// A browser-like subscription: a real P-256 public key and a 16-byte auth secret.
fn subscription(endpoint: &str) -> Value {
    let ua = ES256KeyPair::generate();
    use web_push_native::jwt_simple::algorithms::ECDSAP256PublicKeyLike;
    let p256dh = URL_SAFE_NO_PAD.encode(ua.public_key().public_key().to_bytes_uncompressed());
    let auth = URL_SAFE_NO_PAD.encode([7u8; 16]);
    json!({ "endpoint": endpoint, "expirationTime": null, "keys": { "p256dh": p256dh, "auth": auth } })
}

fn typed(endpoint: &str) -> PushSubscription {
    let v = subscription(endpoint);
    PushSubscription {
        endpoint: endpoint.to_owned(),
        keys: PushKeys {
            p256dh: v["keys"]["p256dh"].as_str().unwrap().to_owned(),
            auth: v["keys"]["auth"].as_str().unwrap().to_owned(),
        },
    }
}

async fn devices(state: &AppState, app: &axum::Router, raw: &str) -> i64 {
    let res = send(
        app,
        as_session(state, raw, Method::GET, "/api/me/push", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    res.body["devices"].as_i64().unwrap()
}

async fn subscribe(state: &AppState, app: &axum::Router, raw: &str, body: Value) -> StatusCode {
    let res = send(
        app,
        as_session(state, raw, Method::POST, "/api/me/push", Some(body)),
    )
    .await;
    res.status
}

fn test_notice(user_id: &str, dedupe: Option<&str>) -> Notice {
    Notice {
        user_id: user_id.to_owned(),
        payload: Payload {
            kind: NoticeKind::Test,
            url: "/account".to_owned(),
            ..Payload::default()
        },
        dedupe: dedupe.map(str::to_owned),
    }
}

async fn outbox(pool: &SqlitePool) -> Vec<(String, i64, Option<i64>)> {
    sqlx::query_as("SELECT payload, attempts, sent_at FROM push_outbox ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn vapid_keys_are_made_once_and_kept() {
    let (state, pool, ..) = with_admin().await;
    let first = vapid_keys(&state.store).await.unwrap().public_b64();
    let second = vapid_keys(&state.store).await.unwrap().public_b64();
    let other = vapid_keys(&Store::new(pool.clone()))
        .await
        .unwrap()
        .public_b64();
    assert_eq!(first, second);
    assert_eq!(first, other);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM push_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn a_second_vapid_read_does_not_write() {
    let (state, pool, ..) = with_admin().await;
    let first = vapid_keys(&state.store).await.unwrap().public_b64();
    // From here any insert into push_keys aborts, even an `INSERT OR IGNORE`.
    sqlx::query(
        "CREATE TRIGGER no_key_writes BEFORE INSERT ON push_keys \
         BEGIN SELECT RAISE(ABORT, 'push_keys written'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let second = vapid_keys(&state.store).await.unwrap().public_b64();
    assert_eq!(first, second);
}

#[tokio::test]
async fn public_key_is_a_65_byte_uncompressed_point() {
    let (state, _pool, app, ..) = with_admin().await;
    let key = vapid_keys(&state.store).await.unwrap().public_b64();
    let bytes = URL_SAFE_NO_PAD.decode(&key).unwrap();
    assert_eq!(bytes.len(), 65);
    assert_eq!(bytes[0], 0x04);
    // The route needs no account and serves the same key.
    let res = send(&app, req(Method::GET, "/api/push/key", None, &[])).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, json!({ "key": key }));
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
}

#[tokio::test]
async fn subscribe_requires_session() {
    let (state, _pool, app, admin, _raw) = with_admin().await;
    let body = subscription("https://push.example.test/abc");
    let res = send(
        &app,
        req(Method::POST, "/api/me/push", Some(body.clone()), &[]),
    )
    .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let token = device_token(&state.store, &admin).await;
    for (method, path, body) in [
        (Method::POST, "/api/me/push", Some(body.clone())),
        (Method::GET, "/api/me/push", None),
        (
            Method::DELETE,
            "/api/me/push",
            Some(json!({ "endpoint": "https://push.example.test/abc" })),
        ),
        (Method::POST, "/api/me/push/test", None),
    ] {
        let res = send(&app, as_device(&token, method, path, body)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn subscription_endpoint_must_be_https_public_host() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    let long = format!("https://push.example.test/{}", "a".repeat(1025 - 26));
    assert_eq!(long.chars().count(), 1025);
    let mut bad_key = subscription("https://push.example.test/abc");
    bad_key["keys"]["p256dh"] = json!(URL_SAFE_NO_PAD.encode([4u8; 10]));
    let mut bad_auth = subscription("https://push.example.test/abc");
    bad_auth["keys"]["auth"] = json!(URL_SAFE_NO_PAD.encode([1u8; 15]));
    let mut bodies: Vec<Value> = [
        "http://push.example.test/x",
        "https://127.0.0.1/x",
        "https://[::1]/x",
        "https://localhost/x",
        "https://a.localhost/x",
        "https://2130706433/x",
        "https://router/x",
        "https://router./x",
        "https://push.example.test:8443/x",
        "https://push.example.test:22/x",
        "https://user@push.example.test/x",
        "not a url",
        long.as_str(),
    ]
    .into_iter()
    .map(subscription)
    .collect();
    bodies.push(bad_key);
    bodies.push(bad_auth);
    for body in bodies {
        let res = send(
            &app,
            as_session(
                &state,
                &raw,
                Method::POST,
                "/api/me/push",
                Some(body.clone()),
            ),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(
            res.body,
            json!({ "error": "validation", "field": "endpoint" }),
            "{body}"
        );
    }
    assert_eq!(devices(&state, &app, &raw).await, 0);
    let ok = subscription("https://push.example.test/abc");
    assert_eq!(
        subscribe(&state, &app, &raw, ok).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(devices(&state, &app, &raw).await, 1);
    // The default port, written out, is fine.
    let explicit = subscription("https://push.example.test:443/def");
    assert_eq!(
        subscribe(&state, &app, &raw, explicit).await,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn resubscribing_updates_not_duplicates() {
    let (state, _pool, app, _admin, raw) = with_admin().await;
    for _ in 0..2 {
        let body = subscription("https://push.example.test/abc");
        assert_eq!(
            subscribe(&state, &app, &raw, body).await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(devices(&state, &app, &raw).await, 1);
    // Unsubscribing removes it; a second unsubscribe is a no-op.
    for _ in 0..2 {
        let res = send(
            &app,
            as_session(
                &state,
                &raw,
                Method::DELETE,
                "/api/me/push",
                Some(json!({ "endpoint": "https://push.example.test/abc" })),
            ),
        )
        .await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
    }
    assert_eq!(devices(&state, &app, &raw).await, 0);
}

#[tokio::test]
async fn endpoint_moves_to_the_new_user() {
    let (state, _pool, app, _admin, a) = with_admin().await;
    let (_, b) = invited_user(&state, &app, &a, "rui@x.test", "Rui").await;
    let endpoint = "https://push.example.test/shared";
    assert_eq!(
        subscribe(&state, &app, &a, subscription(endpoint)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        subscribe(&state, &app, &b, subscription(endpoint)).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(devices(&state, &app, &a).await, 0);
    assert_eq!(devices(&state, &app, &b).await, 1);
    // A's unsubscribe of B's endpoint changes nothing.
    let res = send(
        &app,
        as_session(
            &state,
            &a,
            Method::DELETE,
            "/api/me/push",
            Some(json!({ "endpoint": endpoint })),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(devices(&state, &app, &b).await, 1);
}

#[tokio::test]
async fn eleventh_device_drops_the_oldest() {
    let (state, pool, _app, admin, _raw) = with_admin().await;
    for i in 0..=push::MAX_DEVICES_PER_USER {
        let endpoint = format!("https://push.example.test/d{i}");
        state
            .store
            .save_push_subscription(&admin, &typed(&endpoint), 1_000 + i64::try_from(i).unwrap())
            .await
            .unwrap();
    }
    assert_eq!(
        state.store.push_device_count(&admin).await.unwrap(),
        push::MAX_DEVICES_PER_USER
    );
    let oldest: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE endpoint = ?")
            .bind("https://push.example.test/d0")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(oldest, 0);
}

#[tokio::test]
async fn queued_notice_goes_to_every_device() {
    let (mut state, pool, _app, admin, _raw) = with_admin().await;
    let fake = FakeTransport::answering(&[]);
    with_fake(&mut state, &fake);
    let endpoints = [
        "https://push.example.test/one",
        "https://push.example.test/two",
    ];
    for endpoint in endpoints {
        state
            .store
            .save_push_subscription(&admin, &typed(endpoint), 1_000)
            .await
            .unwrap();
    }
    state
        .store
        .queue(&[test_notice(&admin, None)], 1_000)
        .await
        .unwrap();
    let report = send_pending(&state, 1_010).await;
    assert_eq!(
        report,
        SendReport {
            sent: 2,
            gone: 0,
            failed: 0
        }
    );
    let mut uris: Vec<String> = Vec::new();
    {
        let requests = fake.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for r in requests.iter() {
            assert_eq!(r.method(), Method::POST);
            assert_eq!(r.headers()["content-encoding"], "aes128gcm");
            assert!(
                r.headers()["authorization"]
                    .to_str()
                    .unwrap()
                    .starts_with("vapid t=")
            );
            assert_eq!(r.headers()["ttl"], "86400");
            assert!(!r.body().is_empty());
            uris.push(r.uri().to_string());
        }
    }
    uris.sort();
    assert_eq!(uris, endpoints);
    assert_eq!(outbox(&pool).await[0].2, Some(1_010));
    let ok: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE last_ok_at = 1010")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ok, 2);
    // Nothing left to send.
    assert_eq!(send_pending(&state, 1_020).await, SendReport::default());
}

#[tokio::test]
async fn gone_subscription_is_deleted() {
    let (mut state, pool, _app, admin, _raw) = with_admin().await;
    let fake = FakeTransport::answering(&[410]);
    with_fake(&mut state, &fake);
    state
        .store
        .save_push_subscription(&admin, &typed("https://push.example.test/old"), 1_000)
        .await
        .unwrap();
    state
        .store
        .queue(&[test_notice(&admin, None)], 1_000)
        .await
        .unwrap();
    let report = send_pending(&state, 1_010).await;
    assert_eq!(
        report,
        SendReport {
            sent: 0,
            gone: 1,
            failed: 0
        }
    );
    assert_eq!(state.store.push_device_count(&admin).await.unwrap(), 0);
    // No devices left: the row is done.
    assert_eq!(outbox(&pool).await[0].2, Some(1_010));
}

#[tokio::test]
async fn failed_send_is_retried_then_dropped() {
    let (mut state, pool, _app, admin, _raw) = with_admin().await;
    let fake = FakeTransport::answering(&[500, 500, 500]);
    with_fake(&mut state, &fake);
    state
        .store
        .save_push_subscription(&admin, &typed("https://push.example.test/flaky"), 1_000)
        .await
        .unwrap();
    state
        .store
        .queue(&[test_notice(&admin, None)], 1_000)
        .await
        .unwrap();
    for (run, attempts) in [(1, 1), (2, 2)] {
        let report = send_pending(&state, 1_000 + run).await;
        assert_eq!(report.failed, 1);
        let row = &outbox(&pool).await[0];
        assert_eq!((row.1, row.2), (attempts, None));
    }
    let report = send_pending(&state, 1_003).await;
    assert_eq!(report.failed, 1);
    let row = &outbox(&pool).await[0];
    assert_eq!((row.1, row.2), (MAX_ATTEMPTS, Some(1_003)));
    assert_eq!(fake.requests.lock().unwrap().len(), 3);
    // The subscription stays: only 404/410 remove it.
    assert_eq!(state.store.push_device_count(&admin).await.unwrap(), 1);
}

#[tokio::test]
async fn dedupe_key_queues_once() {
    let (state, pool, _app, admin, _raw) = with_admin().await;
    let queued = state
        .store
        .queue(
            &[
                test_notice(&admin, Some("game:1")),
                test_notice(&admin, Some("game:1")),
            ],
            1_000,
        )
        .await
        .unwrap();
    assert_eq!(queued, 1);
    let again = state
        .store
        .queue(&[test_notice(&admin, Some("game:1"))], 1_001)
        .await
        .unwrap();
    assert_eq!(again, 0);
    // No key: never deduplicated.
    let plain = state
        .store
        .queue(
            &[test_notice(&admin, None), test_notice(&admin, None)],
            1_002,
        )
        .await
        .unwrap();
    assert_eq!(plain, 2);
    assert_eq!(outbox(&pool).await.len(), 3);
}

#[tokio::test]
async fn test_route_queues_a_test_notice() {
    let (state, pool, app, admin, raw) = with_admin().await;
    let res = send(
        &app,
        as_session(&state, &raw, Method::POST, "/api/me/push/test", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::ACCEPTED);
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT user_id, payload FROM push_outbox")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, admin);
    assert_eq!(rows[0].1, r#"{"kind":"test","url":"/account"}"#);
}

#[tokio::test]
async fn test_route_is_limited_to_five_per_ten_minutes_per_user() {
    let (state, pool, app, _admin, raw) = with_admin().await;
    for i in 0..5 {
        let res = send(
            &app,
            as_session(&state, &raw, Method::POST, "/api/me/push/test", None),
        )
        .await;
        assert_eq!(res.status, StatusCode::ACCEPTED, "call {i}");
    }
    let res = send(
        &app,
        as_session(&state, &raw, Method::POST, "/api/me/push/test", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(res.body["error"], json!("too_many_requests"));
    let retry = res.body["retry_after_secs"].as_u64().unwrap();
    assert!((1..=600).contains(&retry), "{retry}");
    assert_eq!(
        res.headers[axum::http::header::RETRY_AFTER]
            .to_str()
            .unwrap(),
        retry.to_string()
    );
    // The refused call queued nothing.
    assert_eq!(outbox(&pool).await.len(), 5);
}

/// Answers only once both devices of a row are in flight at the same time; a sender that goes one
/// device at a time never gets the second request out, and the wait times out.
struct MeetingTransport {
    barrier: tokio::sync::Barrier,
}

impl PushTransport for MeetingTransport {
    fn send(&self, _req: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<u16, String>> {
        Box::pin(async move {
            let met =
                tokio::time::timeout(std::time::Duration::from_secs(5), self.barrier.wait()).await;
            met.map(|_| 201).map_err(|_| "not concurrent".to_owned())
        })
    }
}

#[tokio::test]
async fn a_rows_devices_are_attempted_together() {
    let (mut state, _pool, _app, admin, _raw) = with_admin().await;
    let transport: Arc<dyn PushTransport> = Arc::new(MeetingTransport {
        barrier: tokio::sync::Barrier::new(2),
    });
    state.push = Arc::new(Pusher::new(transport));
    for endpoint in [
        "https://push.example.test/one",
        "https://push.example.test/two",
    ] {
        state
            .store
            .save_push_subscription(&admin, &typed(endpoint), 1_000)
            .await
            .unwrap();
    }
    state
        .store
        .queue(&[test_notice(&admin, None)], 1_000)
        .await
        .unwrap();
    let report = send_pending(&state, 1_010).await;
    assert_eq!(
        report,
        SendReport {
            sent: 2,
            gone: 0,
            failed: 0
        }
    );
}
