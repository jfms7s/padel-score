//! Web push: the server's VAPID key, each device's push subscription, and the outbox of notices
//! waiting to be sent.
//!
//! A notice is queued in the same transaction as the change it announces ([`queue_in`]) and sent
//! later by [`run_sender`], so a push is never lost to a crash and never sent for a change that
//! rolled back. The text is made on the phone: a [`Payload`] carries a kind, a same-origin path
//! and instants, and the service worker formats them in the device's time zone.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::future::{BoxFuture, join_all};
use http::Uri;
use serde::{Deserialize, Serialize};
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike, ES256KeyPair};
use web_push_native::{Auth, WebPushBuilder, p256};

use crate::AppState;
use crate::config::Config;
use crate::store::{Store, StoreError, Tx};

/// A user keeps at most this many devices; a new one past it drops the oldest.
pub const MAX_DEVICES_PER_USER: usize = 10;
/// A notice no device took is dropped after this many failed passes.
pub const MAX_ATTEMPTS: i64 = 3;
/// Outbox rows older than this (two days), sent or not, are pruned by the tick.
pub const OUTBOX_KEEP_SECS: i64 = 172_800;

/// How long a push service keeps a notice for an offline device.
const TTL: Duration = Duration::from_secs(86_400);
const MAX_ENDPOINT_CHARS: usize = 1024;
/// Outbox rows taken per pass.
const BATCH: i64 = 100;
/// How often the sender looks for work without being woken.
const SEND_INTERVAL: Duration = Duration::from_secs(30);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const P256DH_BYTES: usize = 65;
const AUTH_BYTES: usize = 16;

/// The server's VAPID key pair. The private half never leaves the database.
pub struct VapidKeys {
    pair: ES256KeyPair,
    public: String,
}

impl VapidKeys {
    fn from_pair(pair: ES256KeyPair) -> VapidKeys {
        let public = URL_SAFE_NO_PAD.encode(pair.public_key().public_key().to_bytes_uncompressed());
        VapidKeys { pair, public }
    }

    /// The 65-byte uncompressed public point in base64url without padding: the browser's
    /// `applicationServerKey`.
    pub fn public_b64(&self) -> String {
        self.public.clone()
    }

    pub fn key_pair(&self) -> &ES256KeyPair {
        &self.pair
    }
}

/// The VAPID key pair, made on first use. Reading is the common case and takes no write lock; only
/// a missing row generates a key. Two first starts racing agree: each `INSERT OR IGNORE`s its
/// own key and then reads whichever row won.
pub async fn vapid_keys(store: &Store) -> Result<VapidKeys, StoreError> {
    let stored = match read_vapid_key(store).await? {
        Some(stored) => stored,
        None => {
            let fresh = URL_SAFE_NO_PAD.encode(ES256KeyPair::generate().to_bytes());
            sqlx::query(
                "INSERT OR IGNORE INTO push_keys (id, private_key, created_at) VALUES (1, ?, ?)",
            )
            .bind(fresh)
            .bind(crate::api::now())
            .execute(store.pool())
            .await?;
            read_vapid_key(store)
                .await?
                .ok_or_else(|| StoreError::Corrupt("stored VAPID key: row missing".to_owned()))?
        }
    };
    let pair = URL_SAFE_NO_PAD
        .decode(stored)
        .map_err(|e| e.to_string())
        .and_then(|raw| ES256KeyPair::from_bytes(&raw).map_err(|e| e.to_string()))
        .map_err(|e| StoreError::Corrupt(format!("stored VAPID key: {e}")))?;
    Ok(VapidKeys::from_pair(pair))
}

async fn read_vapid_key(store: &Store) -> Result<Option<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT private_key FROM push_keys WHERE id = 1")
            .fetch_optional(store.pool())
            .await?,
    )
}

/// The VAPID `sub` claim: `PUBLIC_ORIGIN` when set, else a placeholder `mailto:`.
pub fn vapid_subject(config: &Config) -> String {
    config
        .public_origin
        .clone()
        .unwrap_or_else(|| "mailto:push@localhost".to_owned())
}

/// A browser's `subscription.toJSON()`; fields it adds (`expirationTime`) are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscription {
    pub endpoint: String,
    pub keys: PushKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Whether `endpoint` may be stored and later POSTed to: `https` on the default port (443, as
/// every browser push service is), at most 1024 chars, a dotted host name (no single label such
/// as `router`, no IP literal in any notation, no `localhost`) and no userinfo. Any failure is
/// `InvalidField("endpoint")`. Where the name resolves is checked at connect time, by
/// [`GlobalOnly`].
pub fn check_endpoint(endpoint: &str) -> Result<(), StoreError> {
    let bad = || StoreError::InvalidField("endpoint");
    if endpoint.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(bad());
    }
    let uri: Uri = endpoint.parse().map_err(|_| bad())?;
    if uri.scheme_str() != Some("https") {
        return Err(bad());
    }
    let authority = uri.authority().ok_or_else(bad)?;
    if authority.as_str().contains('@') {
        return Err(bad());
    }
    if !matches!(uri.port_u16(), None | Some(443)) {
        return Err(bad());
    }
    let host = authority.host().trim_end_matches('.').to_ascii_lowercase();
    let last_label = host.rsplit('.').next().unwrap_or_default();
    // A numeric last label is an IPv4 address in some notation (`2130706433`, `0x7f.1`).
    let numeric = last_label.bytes().all(|b| b.is_ascii_digit()) || last_label.starts_with("0x");
    if host.is_empty()
        || !host.contains('.')
        || host.starts_with('[')
        || host == "localhost"
        || host.ends_with(".localhost")
        || numeric
        || host.parse::<IpAddr>().is_ok()
    {
        return Err(bad());
    }
    Ok(())
}

/// A subscription's keys decoded: the browser's P-256 point and its auth secret.
fn decode_keys(keys: &PushKeys) -> Option<(p256::PublicKey, Auth)> {
    let point = URL_SAFE_NO_PAD.decode(&keys.p256dh).ok()?;
    let secret = URL_SAFE_NO_PAD.decode(&keys.auth).ok()?;
    if point.len() != P256DH_BYTES {
        return None;
    }
    let secret = <[u8; AUTH_BYTES]>::try_from(secret).ok()?;
    let public = p256::PublicKey::from_sec1_bytes(&point).ok()?;
    Some((public, secret.into()))
}

impl Store {
    /// Save (or refresh) a device's subscription. The endpoint is the key, so a phone that signs
    /// in as someone else moves to them. Past [`MAX_DEVICES_PER_USER`] the user's oldest goes.
    pub async fn save_push_subscription(
        &self,
        user_id: &str,
        sub: &PushSubscription,
        now: i64,
    ) -> Result<(), StoreError> {
        check_endpoint(&sub.endpoint)?;
        if decode_keys(&sub.keys).is_none() {
            return Err(StoreError::InvalidField("endpoint"));
        }
        let mut tx = self.begin().await?;
        sqlx::query(
            "INSERT INTO push_subscriptions (endpoint, user_id, p256dh, auth, created_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(endpoint) DO UPDATE SET user_id = excluded.user_id, \
             p256dh = excluded.p256dh, auth = excluded.auth, created_at = excluded.created_at, \
             last_ok_at = NULL",
        )
        .bind(&sub.endpoint)
        .bind(user_id)
        .bind(&sub.keys.p256dh)
        .bind(&sub.keys.auth)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let keep = i64::try_from(MAX_DEVICES_PER_USER).unwrap_or(i64::MAX);
        sqlx::query(
            "DELETE FROM push_subscriptions WHERE endpoint IN ( \
               SELECT endpoint FROM push_subscriptions WHERE user_id = ? AND endpoint <> ? \
               ORDER BY created_at DESC, rowid DESC LIMIT -1 OFFSET ?)",
        )
        .bind(user_id)
        .bind(&sub.endpoint)
        .bind(keep - 1)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Remove the caller's own subscription; absent, or another user's, changes nothing.
    pub async fn delete_push_subscription(
        &self,
        user_id: &str,
        endpoint: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ? AND user_id = ?")
            .bind(endpoint)
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn push_device_count(&self, user_id: &str) -> Result<usize, StoreError> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE user_id = ?")
                .bind(user_id)
                .fetch_one(self.pool())
                .await?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Queue notices on their own; see [`queue_in`] for queueing inside a command.
    pub async fn queue(&self, notices: &[Notice], now: i64) -> Result<usize, StoreError> {
        let mut tx = self.begin().await?;
        let queued = queue_in(&mut tx, notices, now).await?;
        tx.commit().await?;
        Ok(queued)
    }

    /// Delete outbox rows created before `cutoff`, sent or not. Returns how many.
    pub async fn prune_push_outbox(&self, cutoff: i64) -> Result<usize, StoreError> {
        let done = sqlx::query("DELETE FROM push_outbox WHERE created_at < ?")
            .bind(cutoff)
            .execute(self.pool())
            .await?;
        Ok(usize::try_from(done.rows_affected()).unwrap_or(usize::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    #[default]
    Test,
    GameFormed,
    MovedIn,
    PlayerLeft,
    PlaceGiven,
    BoardDeleted,
    SwapAsked,
    SwapAnswered,
    TimeAsked,
    TimeAnswered,
    TimeChanged,
    EventCancelled,
    EventChanged,
}

/// What a notification says, as data. `url` is always a same-origin path (`/b/{token}`,
/// `/j/{token}`, `/calendar`, `/account`); the other fields depend on the kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Payload {
    pub kind: NoticeKind,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_end: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
}

/// One notice for one user. With a `dedupe` key it is queued at most once while the earlier row
/// is still kept (the tick prunes rows after [`OUTBOX_KEEP_SECS`]).
#[derive(Debug, Clone)]
pub struct Notice {
    pub user_id: String,
    pub payload: Payload,
    pub dedupe: Option<String>,
}

/// Queue `notices` inside the caller's transaction. A notice whose `dedupe` key is already
/// queued is skipped. Returns how many rows were queued.
pub async fn queue_in(tx: &mut Tx<'_>, notices: &[Notice], now: i64) -> Result<usize, StoreError> {
    let mut queued = 0;
    for notice in notices {
        let payload = serde_json::to_string(&notice.payload)
            .map_err(|e| StoreError::Corrupt(format!("push payload: {e}")))?;
        let done = sqlx::query(
            "INSERT OR IGNORE INTO push_outbox (user_id, payload, dedupe, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&notice.user_id)
        .bind(payload)
        .bind(&notice.dedupe)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        queued += usize::try_from(done.rows_affected()).unwrap_or(0);
    }
    Ok(queued)
}

/// Sends one prepared push request and returns the push service's HTTP status.
pub trait PushTransport: Send + Sync {
    fn send(&self, req: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<u16, String>>;
}

/// The real transport: no redirects, a 5 s connect and 10 s total timeout, no proxy from the
/// environment and only globally routable addresses ([`GlobalOnly`]), since the endpoint is a URL
/// a client chose.
pub struct ReqwestTransport {
    client: Result<reqwest::Client, String>,
}

impl ReqwestTransport {
    pub fn new() -> ReqwestTransport {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(SEND_TIMEOUT)
            .no_proxy()
            .dns_resolver(Arc::new(GlobalOnly))
            .build()
            .map_err(|e| e.without_url().to_string());
        ReqwestTransport { client }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        ReqwestTransport::new()
    }
}

impl PushTransport for ReqwestTransport {
    fn send(&self, req: http::Request<Vec<u8>>) -> BoxFuture<'_, Result<u16, String>> {
        Box::pin(async move {
            let client = self.client.as_ref().map_err(Clone::clone)?;
            let req = reqwest::Request::try_from(req).map_err(|e| e.without_url().to_string())?;
            let res = client
                .execute(req)
                .await
                .map_err(|e| e.without_url().to_string())?;
            Ok(res.status().as_u16())
        })
    }
}

/// Whether `ip` is a public, globally routable address a push service can have. Refuses the
/// private, loopback, link-local, shared (CGNAT), documentation, benchmarking, multicast,
/// reserved and unspecified ranges of both families, and judges a v4-mapped, NAT64 or 6to4 v6
/// address by the v4 address inside it.
pub fn is_global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => is_global_v6(v6),
    }
}

fn is_global_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..128).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_global_v6(ip: Ipv6Addr) -> bool {
    let [s0, s1, s2, s3, s4, s5, s6, s7] = ip.segments();
    let embedded = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        is_global_v4(Ipv4Addr::new(a, b, c, d))
    };
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_global_v4(v4);
    }
    // NAT64 (64:ff9b::/96) and the v4-compatible ::/96 carry a v4 address in the last 32 bits.
    if (s0, s1, s2, s3, s4, s5) == (0x64, 0xff9b, 0, 0, 0, 0) {
        return embedded(s6, s7);
    }
    // 6to4 (2002::/16) carries one right after the prefix.
    if s0 == 0x2002 {
        return embedded(s1, s2);
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || (s0 & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (s0 & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (s0 & 0xffc0) == 0xfec0 // site-local fec0::/10
        || (s0 & 0xff00) == 0xff00 // multicast ff00::/8
        || (s0 == 0x2001 && s1 == 0x0db8) // documentation 2001:db8::/32
        || (s0 == 0 && s1 == 0 && s2 == 0 && s3 == 0 && s4 == 0 && s5 == 0)) // ::/96
}

/// A DNS resolver that drops every address that is not [`is_global`] and fails when none is left.
/// It runs at connect time, so a name that points inward, or is rebound to do so after
/// [`check_endpoint`], still cannot reach the local network.
pub struct GlobalOnly;

/// `addrs` without the non-global ones, or an error when nothing is left.
fn keep_global(addrs: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, String> {
    let kept: Vec<SocketAddr> = addrs
        .into_iter()
        .filter(|addr| is_global(addr.ip()))
        .collect();
    if kept.is_empty() {
        Err("the host resolves to no public address".to_owned())
    } else {
        Ok(kept)
    }
}

impl reqwest::dns::Resolve for GlobalOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            let kept = keep_global(found)?;
            Ok(Box::new(kept.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

pub struct Pusher {
    pub transport: Arc<dyn PushTransport>,
    wake: tokio::sync::Notify,
}

impl Pusher {
    pub fn new(transport: Arc<dyn PushTransport>) -> Pusher {
        Pusher {
            transport,
            wake: tokio::sync::Notify::new(),
        }
    }

    /// Ask the sender to look at the outbox now.
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

/// What one pass did, counted in device sends.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SendReport {
    pub sent: usize,
    pub gone: usize,
    pub failed: usize,
}

/// How one push to one device ended.
enum Outcome {
    Sent,
    Gone,
    Failed,
}

/// One pass: send up to 100 unsent rows, oldest first, to every device of their user.
pub async fn send_pending(state: &AppState, now: i64) -> SendReport {
    send_batch(state, now).await.0
}

/// [`send_pending`], and how many outbox rows it took.
async fn send_batch(state: &AppState, now: i64) -> (SendReport, i64) {
    let mut report = SendReport::default();
    let rows: Vec<(i64, String, String, i64)> = match sqlx::query_as(
        "SELECT id, user_id, payload, attempts FROM push_outbox WHERE sent_at IS NULL \
         ORDER BY created_at, id LIMIT ?",
    )
    .bind(BATCH)
    .fetch_all(state.store.pool())
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "push outbox read failed");
            return (report, 0);
        }
    };
    if rows.is_empty() {
        return (report, 0);
    }
    let keys = match vapid_keys(&state.store).await {
        Ok(keys) => keys,
        Err(error) => {
            tracing::error!(%error, "push VAPID key unavailable");
            return (report, 0);
        }
    };
    let subject = vapid_subject(&state.config);
    let taken = i64::try_from(rows.len()).unwrap_or(BATCH);
    for (id, user_id, payload, attempts) in rows {
        let subs = match user_subscriptions(&state.store, &user_id).await {
            Ok(subs) => subs,
            Err(error) => {
                tracing::error!(%error, "push subscriptions read failed");
                continue;
            }
        };
        let (mut sent, mut gone, mut failed) = (0, 0, 0);
        // A row's devices go out together, so a stalled push service costs one timeout, not one
        // per device.
        let outcomes = join_all(subs.iter().map(|(endpoint, keys_of_device)| {
            push_one(state, &keys, &subject, endpoint, keys_of_device, &payload)
        }))
        .await;
        for ((endpoint, _), outcome) in subs.iter().zip(outcomes) {
            match outcome {
                Outcome::Sent => {
                    sent += 1;
                    mark_ok(&state.store, endpoint, now).await;
                }
                Outcome::Gone => {
                    gone += 1;
                    drop_endpoint(&state.store, endpoint).await;
                }
                Outcome::Failed => failed += 1,
            }
        }
        report.sent += sent;
        report.gone += gone;
        report.failed += failed;
        // Done once a device took it (a retry would repeat it there), when no device is left,
        // or after the last allowed attempt.
        let remaining = subs.len().saturating_sub(gone);
        let retry = sent == 0 && remaining > 0;
        let attempts = if retry { attempts + 1 } else { attempts };
        let done = !retry || attempts >= MAX_ATTEMPTS;
        finish_row(&state.store, id, done.then_some(now), attempts).await;
    }
    (report, taken)
}

async fn user_subscriptions(
    store: &Store,
    user_id: &str,
) -> Result<Vec<(String, PushKeys)>, StoreError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = ? \
         ORDER BY created_at, rowid",
    )
    .bind(user_id)
    .fetch_all(store.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(endpoint, p256dh, auth)| (endpoint, PushKeys { p256dh, auth }))
        .collect())
}

/// Encrypt `payload` for one device and post it. Neither the endpoint nor the keys are logged.
async fn push_one(
    state: &AppState,
    vapid: &VapidKeys,
    subject: &str,
    endpoint: &str,
    device: &PushKeys,
    payload: &str,
) -> Outcome {
    let Some((public, auth)) = decode_keys(device) else {
        tracing::warn!("push: stored subscription keys are unusable");
        return Outcome::Failed;
    };
    let Ok(uri) = endpoint.parse::<Uri>() else {
        tracing::warn!("push: stored endpoint is unusable");
        return Outcome::Failed;
    };
    let built = WebPushBuilder::new(uri, public, auth)
        .with_valid_duration(TTL)
        .with_vapid(vapid.key_pair(), subject)
        .build(payload);
    let request = match built {
        Ok(request) => request,
        Err(error) => {
            tracing::error!(%error, "push: cannot build the request");
            return Outcome::Failed;
        }
    };
    match state.push.transport.send(request).await {
        Ok(200..=299) => Outcome::Sent,
        Ok(404 | 410) => Outcome::Gone,
        Ok(status) => {
            tracing::warn!(status, "push service refused a notice");
            Outcome::Failed
        }
        Err(error) => {
            tracing::warn!(%error, "push send failed");
            Outcome::Failed
        }
    }
}

async fn mark_ok(store: &Store, endpoint: &str, now: i64) {
    let done = sqlx::query("UPDATE push_subscriptions SET last_ok_at = ? WHERE endpoint = ?")
        .bind(now)
        .bind(endpoint)
        .execute(store.pool())
        .await;
    if let Err(error) = done {
        tracing::error!(%error, "push subscription update failed");
    }
}

async fn drop_endpoint(store: &Store, endpoint: &str) {
    let done = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?")
        .bind(endpoint)
        .execute(store.pool())
        .await;
    if let Err(error) = done {
        tracing::error!(%error, "push subscription delete failed");
    }
}

async fn finish_row(store: &Store, id: i64, sent_at: Option<i64>, attempts: i64) {
    let done = sqlx::query("UPDATE push_outbox SET sent_at = ?, attempts = ? WHERE id = ?")
        .bind(sent_at)
        .bind(attempts)
        .bind(id)
        .execute(store.pool())
        .await;
    if let Err(error) = done {
        tracing::error!(%error, "push outbox update failed");
    }
}

/// Send queued notices when woken and every 30 s, until the shutdown token fires. A full batch
/// is followed at once by the next only if it made progress (a device took a notice or was
/// dropped); a full batch that all failed waits for the next wake or interval.
pub async fn run_sender(state: AppState) {
    let mut ticker = tokio::time::interval(SEND_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = state.shutdown.cancelled() => return,
            () = state.push.wake.notified() => {}
            _ = ticker.tick() => {}
        }
        loop {
            let (report, taken) = send_batch(&state, crate::api::now()).await;
            if report.sent + report.gone + report.failed > 0 {
                tracing::info!(
                    sent = report.sent,
                    gone = report.gone,
                    failed = report.failed,
                    "push pass done"
                );
            }
            let progressed = report.sent + report.gone > 0;
            if taken < BATCH || !progressed || state.shutdown.is_cancelled() {
                break;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn v4_private_and_reserved_ranges_are_not_global() {
        for text in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.5",
            "100.64.0.1",
            "100.127.255.255",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_global(ip(text)), "{text}");
        }
    }

    #[test]
    fn v4_public_addresses_are_global_even_at_range_edges() {
        for text in [
            "8.8.8.8",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "192.0.1.1",
            "223.255.255.255",
        ] {
            assert!(is_global(ip(text)), "{text}");
        }
    }

    #[test]
    fn v6_local_and_reserved_ranges_are_not_global() {
        for text in [
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "::10.0.0.1",
        ] {
            assert!(!is_global(ip(text)), "{text}");
        }
    }

    #[test]
    fn v6_public_addresses_are_global() {
        for text in [
            "2607:f8b0:4004:800::200e",
            "2a00:1450:4001::1",
            "2606:4700::1111",
        ] {
            assert!(is_global(ip(text)), "{text}");
        }
    }

    #[test]
    fn v4_mapped_nat64_and_6to4_are_judged_by_the_v4_inside() {
        for text in [
            "::ffff:127.0.0.1",
            "::ffff:10.1.2.3",
            "::ffff:169.254.169.254",
            "64:ff9b::7f00:1",
            "64:ff9b::a00:5",
            "64:ff9b::c0a8:101",
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
        ] {
            assert!(!is_global(ip(text)), "{text}");
        }
        for text in ["::ffff:8.8.8.8", "64:ff9b::808:808", "2002:808:808::1"] {
            assert!(is_global(ip(text)), "{text}");
        }
    }

    #[test]
    fn resolver_keeps_global_addresses_and_fails_when_none_remain() {
        let addr = |text: &str| SocketAddr::new(ip(text), 0);
        let mixed = vec![addr("10.0.0.5"), addr("8.8.8.8"), addr("::1")];
        assert_eq!(keep_global(mixed).unwrap(), vec![addr("8.8.8.8")]);
        assert!(keep_global(vec![addr("127.0.0.1"), addr("fd00::1")]).is_err());
        assert!(keep_global(Vec::new()).is_err());
    }

    #[tokio::test]
    async fn the_real_resolver_refuses_a_name_that_points_inward() {
        use reqwest::dns::Resolve;
        // `localhost` resolves to loopback only, with no network needed.
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(GlobalOnly.resolve(name).await.is_err());
    }
}
