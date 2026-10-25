#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The trusted-proxy client IP, the limiter key, the fixed-window limiter, and the `ClientIp`
//! extractor wired end to end behind `ConnectInfo`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::routing::get;
use http_body_util::BodyExt;
use ipnet::IpNet;
use padel_score_server::api::{ApiError, now};
use padel_score_server::client_ip::{ClientIp, client_ip, rate_key};
use padel_score_server::ratelimit::{MAX_KEYS, RateLimiter};
use padel_score_server::{AppState, Config, Store, db};
use tower::ServiceExt;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn peer(s: &str) -> SocketAddr {
    SocketAddr::new(ip(s), 40_000)
}

fn nets(list: &[&str]) -> Vec<IpNet> {
    list.iter().map(|n| n.parse().unwrap()).collect()
}

fn xff(values: &[&str]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for v in values {
        headers.append("x-forwarded-for", HeaderValue::from_str(v).unwrap());
    }
    headers
}

// Client IP.

#[test]
fn untrusted_peer_ignores_forwarded_for() {
    let trusted = nets(&["10.42.0.0/16"]);
    let headers = xff(&["203.0.113.9"]);
    // A direct client (not a trusted proxy) cannot choose its address with a spoofed header.
    assert_eq!(
        client_ip(peer("198.51.100.7"), &headers, &trusted),
        ip("198.51.100.7")
    );
    // With no trusted proxies at all (the default), the header is never read.
    assert_eq!(client_ip(peer("10.42.0.5"), &headers, &[]), ip("10.42.0.5"));
}

#[test]
fn trusted_proxy_uses_rightmost_untrusted_hop() {
    let trusted = nets(&["10.42.0.0/16", "192.0.2.1/32"]);
    // The client spoofed the leftmost entry; the proxies appended the real hops.
    let headers = xff(&["1.1.1.1, 203.0.113.9, 192.0.2.1"]);
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("203.0.113.9")
    );
    // Several headers are one list, in order.
    let headers = xff(&["1.1.1.1", "203.0.113.9 , 10.42.3.4"]);
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("203.0.113.9")
    );
    // IPv6 hops work the same way.
    let headers = xff(&["2001:db8::1"]);
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("2001:db8::1")
    );
}

#[test]
fn all_trusted_hops_fall_back_to_peer() {
    let trusted = nets(&["10.42.0.0/16"]);
    let headers = xff(&["10.42.9.9, 10.42.8.8"]);
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("10.42.1.2")
    );
    // No header at all: the peer too.
    assert_eq!(
        client_ip(peer("10.42.1.2"), &HeaderMap::new(), &trusted),
        ip("10.42.1.2")
    );
}

#[test]
fn malformed_forwarded_for_falls_back_to_peer() {
    let trusted = nets(&["10.42.0.0/16"]);
    for bad in [
        "not-an-ip",
        "203.0.113.9:8080",
        "203.0.113.9, ",
        "",
        "[2001:db8::1]",
        "203.0.113.9, garbage, 10.42.0.1",
    ] {
        assert_eq!(
            client_ip(peer("10.42.1.2"), &xff(&[bad]), &trusted),
            ip("10.42.1.2"),
            "{bad:?}"
        );
    }
    // A header that is not visible ASCII falls back too.
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        HeaderValue::from_bytes(b"203.0.113.\xff").unwrap(),
    );
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("10.42.1.2")
    );
    // Junk LEFT of the answer is client-written and never reached: it cannot force a fallback.
    let headers = xff(&["garbage, 203.0.113.9, 10.42.0.1"]);
    assert_eq!(
        client_ip(peer("10.42.1.2"), &headers, &trusted),
        ip("203.0.113.9")
    );
}

#[test]
fn ipv4_mapped_is_normalized() {
    let trusted = nets(&["10.42.0.0/16"]);
    // A dual-stack socket reports an IPv4 peer as ::ffff:a.b.c.d; it is still trusted by its v4 net.
    let headers = xff(&["::ffff:203.0.113.9"]);
    assert_eq!(
        client_ip(peer("::ffff:10.42.1.2"), &headers, &trusted),
        ip("203.0.113.9")
    );
    assert_eq!(
        client_ip(peer("::ffff:198.51.100.7"), &HeaderMap::new(), &[]),
        ip("198.51.100.7")
    );
    assert_eq!(rate_key(ip("::ffff:198.51.100.7")), "198.51.100.7");
}

#[test]
fn ipv6_keyed_by_slash_64() {
    let a = rate_key(ip("2001:db8:1:2:aaaa::1"));
    let b = rate_key(ip("2001:db8:1:2:ffff:ffff:ffff:ffff"));
    let c = rate_key(ip("2001:db8:1:3::1"));
    assert_eq!(a, "2001:db8:1:2::/64");
    assert_eq!(a, b);
    assert_ne!(a, c);
    // IPv4 is keyed by the whole address.
    assert_eq!(rate_key(ip("198.51.100.7")), "198.51.100.7");
    assert_ne!(rate_key(ip("198.51.100.7")), rate_key(ip("198.51.100.8")));
}

// The limiter.

#[test]
fn limit_then_retry_after() {
    let limiter = RateLimiter::new(3, 60);
    let t = 1_000_000;
    for _ in 0..3 {
        assert_eq!(limiter.try_acquire("k", t), Ok(()));
    }
    assert_eq!(limiter.try_acquire("k", t), Err(60));
    assert_eq!(limiter.try_acquire("k", t + 45), Err(15));
    // Under a second left still says 1, never 0.
    assert_eq!(limiter.try_acquire("k", t + 59), Err(1));
    // Other keys have their own allowance.
    assert_eq!(limiter.try_acquire("other", t), Ok(()));
}

#[test]
fn window_resets() {
    let limiter = RateLimiter::new(2, 60);
    let t = 1_000_000;
    assert_eq!(limiter.try_acquire("k", t), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 10), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 59), Err(1));
    // The window is fixed from its first call, so it ends at t + 60.
    assert_eq!(limiter.try_acquire("k", t + 60), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 61), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 62), Err(58));
}

#[test]
fn limiter_bounded_at_10000_keys() {
    assert_eq!(MAX_KEYS, 10_000);
    let limiter = RateLimiter::new(1, 60);
    let t = 1_000_000;
    // The first key, at its limit.
    assert_eq!(limiter.try_acquire("first", t), Ok(()));
    assert_eq!(limiter.try_acquire("first", t), Err(60));
    for i in 0..MAX_KEYS {
        assert_eq!(limiter.try_acquire(&format!("k{i}"), t + 1), Ok(()));
    }
    assert_eq!(limiter.len(), MAX_KEYS);
    // The oldest window ("first") was forgotten to make room, so it has a fresh allowance.
    assert_eq!(limiter.try_acquire("first", t + 2), Ok(()));
    assert_eq!(limiter.len(), MAX_KEYS);
    // The newest ones were kept.
    assert_eq!(
        limiter.try_acquire(&format!("k{}", MAX_KEYS - 1), t + 2),
        Err(59)
    );

    // Expired windows are swept on each call and by `sweep`.
    assert_eq!(limiter.try_acquire("late", t + 120), Ok(()));
    assert_eq!(limiter.len(), 1);
    limiter.sweep(t + 300);
    assert_eq!(limiter.len(), 0);
}

#[test]
fn concurrent_acquires_never_exceed_limit() {
    let limiter = Arc::new(RateLimiter::new(10, 60));
    let t = 1_000_000;
    let allowed: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..64)
            .map(|_| {
                let limiter = Arc::clone(&limiter);
                scope.spawn(move || {
                    (0..8)
                        .filter(|_| limiter.try_acquire("login", t).is_ok())
                        .count()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(allowed, 10);
}

#[test]
fn refund_only_affects_current_window() {
    let limiter = RateLimiter::new(2, 60);
    let t = 1_000_000;
    // Reserve-then-refund: a successful login gives its slot back.
    assert_eq!(limiter.try_acquire("k", t), Ok(()));
    limiter.refund("k", t);
    assert_eq!(limiter.try_acquire("k", t + 1), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 1), Ok(()));
    assert_eq!(limiter.try_acquire("k", t + 1), Err(59));

    // A refund never goes below zero: extra refunds buy no extra allowance.
    let limiter = RateLimiter::new(1, 60);
    assert_eq!(limiter.try_acquire("z", t), Ok(()));
    for _ in 0..5 {
        limiter.refund("z", t);
    }
    assert_eq!(limiter.try_acquire("z", t), Ok(()));
    assert_eq!(limiter.try_acquire("z", t), Err(60));

    // A refund for an acquire made in an earlier window does not touch the next window.
    let limiter = RateLimiter::new(1, 60);
    assert_eq!(limiter.try_acquire("w", t), Ok(()));
    assert_eq!(limiter.try_acquire("w", t + 60), Ok(()));
    limiter.refund("w", t);
    assert_eq!(limiter.try_acquire("w", t + 61), Err(59));
    // Nor does a refund for a key with no window create one.
    limiter.refund("absent", t);
    assert_eq!(limiter.len(), 1);
}

// End to end.

async fn state_with(env: &[(&str, &str)]) -> AppState {
    let map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let store = Store::new(db::connect("sqlite::memory:").await.unwrap());
    AppState::new(Arc::new(Config::from_map(&map).unwrap()), store)
}

async fn whoami(ClientIp(ip): ClientIp) -> String {
    ip.to_string()
}

async fn limited(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
) -> Result<String, ApiError> {
    state
        .limits
        .invite_lookup
        .try_acquire(&rate_key(ip), now())
        .map_err(|retry_after| ApiError::TooMany { retry_after })?;
    Ok(ip.to_string())
}

fn app(state: AppState, peer_addr: &str) -> Router {
    Router::new()
        .route("/ip", get(whoami))
        .route("/limited", get(limited))
        .with_state(state)
        .layer(MockConnectInfo(peer(peer_addr)))
}

async fn get_text(
    app: &Router,
    uri: &str,
    forwarded: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut req = Request::get(uri);
    if let Some(f) = forwarded {
        req = req.header("x-forwarded-for", f);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn client_ip_extractor_end_to_end() {
    // Default config: no trusted proxies, the header is ignored.
    let direct = app(state_with(&[]).await, "198.51.100.7");
    let (status, _, body) = get_text(&direct, "/ip", Some("203.0.113.9")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "198.51.100.7");

    // Behind a trusted proxy, the rightmost untrusted hop.
    let state = state_with(&[("TRUSTED_PROXIES", "10.42.0.0/16, 192.0.2.1")]).await;
    let proxied = app(state.clone(), "10.42.0.9");
    let (_, _, body) = get_text(&proxied, "/ip", Some("1.1.1.1, 203.0.113.9, 192.0.2.1")).await;
    assert_eq!(body, "203.0.113.9");

    // The named limiter in AppState refuses the 31st invite lookup a minute with a 429.
    for _ in 0..30 {
        let (status, _, _) = get_text(&proxied, "/limited", Some("203.0.113.9")).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, headers, body) = get_text(&proxied, "/limited", Some("203.0.113.9")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let retry: u64 = headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry), "{retry}");
    assert!(body.contains("too_many_requests"), "{body}");
    // A different client behind the same proxy is not affected.
    let (status, _, _) = get_text(&proxied, "/limited", Some("203.0.113.10")).await;
    assert_eq!(status, StatusCode::OK);
}
