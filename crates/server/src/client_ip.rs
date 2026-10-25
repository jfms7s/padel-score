//! The client's IP address, as the rate limiters see it.
//!
//! The TCP peer (`ConnectInfo`) is the client unless it is a trusted reverse proxy
//! (`TRUSTED_PROXIES`). Behind one, `X-Forwarded-For` is walked from the right, the end the
//! proxies append to, and the first hop that is not itself a trusted proxy is the client. Entries
//! further left were written by the client and are never read, so a spoofed header buys nothing.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use ipnet::IpNet;

use crate::AppState;
use crate::api::{ApiError, internal};

/// The client IP of `peer`'s request carrying `headers`, given the `trusted` proxy networks.
///
/// - An untrusted peer is the client; `X-Forwarded-For` is ignored completely.
/// - A trusted peer: the rightmost `X-Forwarded-For` hop that is not a trusted proxy. Every
///   `X-Forwarded-For` header counts, in order, as one comma-separated list.
/// - The peer is used when every hop is trusted, the header is missing, or a hop reached by the
///   walk is malformed (not a bare IP; ports and brackets count as malformed).
///
/// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are normalized to IPv4 throughout.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap, trusted: &[IpNet]) -> IpAddr {
    let peer = peer.ip().to_canonical();
    if !is_trusted(peer, trusted) {
        return peer;
    }
    let mut hops = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(value) = value.to_str() else {
            return peer;
        };
        hops.extend(value.split(','));
    }
    for hop in hops.iter().rev() {
        let Ok(ip) = hop.trim().parse::<IpAddr>() else {
            return peer;
        };
        let ip = ip.to_canonical();
        if !is_trusted(ip, trusted) {
            return ip;
        }
    }
    peer
}

fn is_trusted(ip: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|net| net.contains(&ip))
}

/// The key a client IP is rate-limited under: an IPv4 address is its own key; an IPv6 address
/// is keyed by its /64, because one subscriber is routinely handed a whole /64 and could
/// otherwise rotate through it for fresh allowances. IPv4-mapped IPv6 is keyed as IPv4.
pub fn rate_key(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let prefix = u128::from(v6) & !u128::from(u64::MAX);
            format!("{}/64", Ipv6Addr::from(prefix))
        }
    }
}

/// The request's client IP, per [`client_ip`] with the configured `TRUSTED_PROXIES`.
///
/// It needs `ConnectInfo<SocketAddr>`: the server is served with
/// `into_make_service_with_connect_info`, and tests add `MockConnectInfo`. Without it the
/// request fails with a logged 500 rather than lumping every client under one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // The extractor, not a bare extension lookup, so `MockConnectInfo` works in tests.
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(internal)?;
        Ok(ClientIp(client_ip(
            peer,
            &parts.headers,
            &state.config.trusted_proxies,
        )))
    }
}
