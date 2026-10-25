//! Retention: anonymous matches are purged after 30 days with no activity.

use std::time::Duration;

use crate::AppState;
use crate::store::{Store, StoreError};

/// An anonymous match idle for longer than this (30 days) is purged.
pub const ANON_RETENTION_SECS: i64 = 2_592_000;

/// How often the purge runs.
pub const PURGE_INTERVAL: Duration = Duration::from_secs(3600);

/// Delete anonymous matches idle for more than [`ANON_RETENTION_SECS`] as of `now`, returning
/// their ids. Their events and stored command results go with them (`ON DELETE CASCADE`). A match
/// idle for exactly the retention period is kept.
pub async fn purge_inactive(store: &Store, now: i64) -> Result<Vec<String>, StoreError> {
    store
        .purge_anonymous_idle_before(now - ANON_RETENTION_SECS)
        .await
}

/// One purge pass: delete the idle anonymous matches, then end the live streams of the purged
/// ones, and delete the expired sessions, invites, device codes, reset codes and link codes. Owned matches
/// are never purged. Returns how many matches were purged; a failure is logged and counts as
/// none.
pub async fn purge_once(state: &AppState, now: i64) -> usize {
    match state.store.purge_expired_auth(now).await {
        Ok(auth) => tracing::info!(
            sessions = auth.sessions,
            invites = auth.invites,
            device_codes = auth.device_codes,
            resets = auth.resets,
            link_codes = auth.link_codes,
            "expired auth records purged"
        ),
        Err(error) => tracing::error!(%error, "auth records purge failed"),
    }
    match purge_inactive(&state.store, now).await {
        Ok(ids) => {
            for id in &ids {
                state.hub.close(id);
            }
            ids.len()
        }
        Err(error) => {
            tracing::error!(%error, "retention purge failed");
            0
        }
    }
}

/// Purge every [`PURGE_INTERVAL`] until the shutdown token fires. The first pass runs at once.
pub async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(PURGE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = state.shutdown.cancelled() => return,
            _ = ticker.tick() => {}
        }
        let purged = purge_once(&state, unix_now()).await;
        tracing::info!(purged, "retention purge done");
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
