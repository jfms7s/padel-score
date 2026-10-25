//! In-memory fixed-window rate limiting, a port of firstdime's `rate-limiter.ts`.
//!
//! A window opens at a key's first counted call and lasts `window_secs`; up to `limit` calls
//! count in it, then the key is refused until the window ends. The counters live in ONE process
//! and are forgotten on restart: enough for a single-replica server, where they are a brake on
//! guessing and on argon2 work, not what makes a secret safe. A shared counter is the upgrade if
//! the server is ever scaled out.
//!
//! Memory is bounded: at most [`MAX_KEYS`] keys are remembered, the oldest window is forgotten
//! first past that, and expired windows are swept on every call and by [`run_sweeper`].
//!
//! Keys are strings: [`crate::client_ip::rate_key`] for addresses, a normalized email, a user id.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// The most keys one limiter remembers at once.
pub const MAX_KEYS: usize = 10_000;

/// How often [`run_sweeper`] drops expired windows.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
struct Window {
    start: i64,
    count: u32,
    /// Insertion sequence, so windows opened in the same second still order oldest first.
    seq: u64,
}

#[derive(Debug, Default)]
struct Inner {
    windows: HashMap<String, Window>,
    /// Every live window by (start, seq): the first entry is the oldest.
    order: BTreeMap<(i64, u64), String>,
    next_seq: u64,
}

/// A fixed-window limiter: `limit` counted calls per key per `window_secs`.
///
/// Two ways to use it:
/// - [`try_acquire`](RateLimiter::try_acquire) counts every call, allowed or not, and decides
///   (firstdime's `consume`). For routes limited on every request.
/// - [`check`](RateLimiter::check) decides without counting and [`hit`](RateLimiter::hit) counts
///   without deciding. For limits on failures only, like login: check first, hit on failure.
///
/// A refusal is `Err(retry_after)`: whole seconds until the window ends, at least 1. `now` is
/// unix seconds, passed in so tests control the clock. The mutex is never held across an await.
#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window_secs: i64,
    inner: Mutex<Inner>,
}

impl RateLimiter {
    pub fn new(limit: u32, window_secs: i64) -> RateLimiter {
        RateLimiter {
            limit,
            window_secs: window_secs.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Count one call for `key` and decide it: `Ok` for the first `limit` in the window.
    pub fn try_acquire(&self, key: &str, now: i64) -> Result<(), u64> {
        let mut inner = self.lock();
        let window = self.count(&mut inner, key, now);
        if window.count <= self.limit {
            Ok(())
        } else {
            Err(self.retry_after(window.start, now))
        }
    }

    /// Decide whether `key` is still allowed, without counting a call.
    pub fn check(&self, key: &str, now: i64) -> Result<(), u64> {
        let mut inner = self.lock();
        self.sweep_in(&mut inner, now);
        match inner.windows.get(key) {
            Some(window) if self.is_live(window, now) && window.count >= self.limit => {
                Err(self.retry_after(window.start, now))
            }
            _ => Ok(()),
        }
    }

    /// Count one call (say, a failure) for `key`, without deciding.
    pub fn hit(&self, key: &str, now: i64) {
        let mut inner = self.lock();
        self.count(&mut inner, key, now);
    }

    /// Forget every expired window.
    pub fn sweep(&self, now: i64) {
        let mut inner = self.lock();
        self.sweep_in(&mut inner, now);
    }

    /// How many keys are remembered.
    pub fn len(&self) -> usize {
        self.lock().windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // The state is a cache of counters; a panic mid-update cannot leave it unusable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_live(&self, window: &Window, now: i64) -> bool {
        now.saturating_sub(window.start) < self.window_secs
    }

    fn retry_after(&self, start: i64, now: i64) -> u64 {
        let left = start.saturating_add(self.window_secs).saturating_sub(now);
        u64::try_from(left).unwrap_or(0).max(1)
    }

    /// Count a call for `key` in its live window, or open a new one at `now`.
    fn count(&self, inner: &mut Inner, key: &str, now: i64) -> Window {
        self.sweep_in(inner, now);
        if let Some(window) = inner.windows.get_mut(key)
            && self.is_live(window, now)
        {
            window.count = window.count.saturating_add(1);
            return *window;
        }
        if let Some(old) = inner.windows.remove(key) {
            inner.order.remove(&(old.start, old.seq));
        }
        while inner.windows.len() >= MAX_KEYS {
            let Some((_, oldest)) = inner.order.pop_first() else {
                break;
            };
            inner.windows.remove(&oldest);
        }
        let seq = inner.next_seq;
        inner.next_seq = inner.next_seq.wrapping_add(1);
        let window = Window {
            start: now,
            count: 1,
            seq,
        };
        inner.windows.insert(key.to_owned(), window);
        inner.order.insert((now, seq), key.to_owned());
        window
    }

    /// Windows are ordered oldest first, so the expired ones are a prefix: stop at the first
    /// live one.
    fn sweep_in(&self, inner: &mut Inner, now: i64) {
        while let Some(entry) = inner.order.first_entry() {
            let (start, _) = *entry.key();
            if now.saturating_sub(start) < self.window_secs {
                return;
            }
            let key = entry.remove();
            inner.windows.remove(&key);
        }
    }
}

/// The named limiters the account routes use. All windows are one minute.
#[derive(Debug)]
pub struct Limits {
    /// Login FAILURES per client address ([`crate::client_ip::rate_key`]): 10 a minute.
    pub login_ip: RateLimiter,
    /// Login FAILURES per normalized email, from any address: 5 a minute.
    pub login_email: RateLimiter,
    /// `POST /api/device/start` per client address: 10 a minute.
    pub device_start: RateLimiter,
    /// Device-code approvals per user id: 10 a minute.
    pub device_approve: RateLimiter,
    /// `GET /api/invites/{code}` per client address: 30 a minute.
    pub invite_lookup: RateLimiter,
}

impl Limits {
    pub fn new() -> Limits {
        Limits {
            login_ip: RateLimiter::new(10, 60),
            login_email: RateLimiter::new(5, 60),
            device_start: RateLimiter::new(10, 60),
            device_approve: RateLimiter::new(10, 60),
            invite_lookup: RateLimiter::new(30, 60),
        }
    }

    /// Sweep every limiter.
    pub fn sweep(&self, now: i64) {
        for limiter in [
            &self.login_ip,
            &self.login_email,
            &self.device_start,
            &self.device_approve,
            &self.invite_lookup,
        ] {
            limiter.sweep(now);
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::new()
    }
}

/// Sweep `limits` every [`SWEEP_INTERVAL`] until `shutdown` fires, so idle keys are dropped even
/// when no calls come in.
pub async fn run_sweeper(limits: Arc<Limits>, shutdown: CancellationToken) {
    let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            _ = ticker.tick() => {}
        }
        limits.sweep(crate::api::now());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn hit_and_try_acquire_share_one_count() {
        let limiter = RateLimiter::new(2, 60);
        limiter.hit("k", 0);
        assert_eq!(limiter.try_acquire("k", 0), Ok(()));
        assert_eq!(limiter.try_acquire("k", 0), Err(60));
    }

    #[test]
    fn a_reopened_window_moves_to_the_back_of_the_eviction_order() {
        let limiter = RateLimiter::new(1, 60);
        limiter.hit("a", 0);
        limiter.hit("b", 1);
        // "a" expires and reopens later than "b": "b" is now the oldest.
        limiter.hit("a", 60);
        let inner = limiter.lock();
        let order: Vec<&str> = inner.order.values().map(String::as_str).collect();
        assert_eq!(order, ["b", "a"]);
        assert_eq!(inner.windows.len(), inner.order.len());
    }

    #[test]
    fn named_limits_match_the_plan() {
        let limits = Limits::new();
        let cases = [
            (&limits.login_ip, 10),
            (&limits.login_email, 5),
            (&limits.device_start, 10),
            (&limits.device_approve, 10),
            (&limits.invite_lookup, 30),
        ];
        for (limiter, limit) in cases {
            assert_eq!(limiter.limit, limit);
            assert_eq!(limiter.window_secs, 60);
        }
    }

    #[tokio::test]
    async fn sweeper_stops_on_shutdown() {
        let limits = Arc::new(Limits::new());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run_sweeper(limits, shutdown.clone()));
        shutdown.cancel();
        task.await.unwrap();
    }
}
