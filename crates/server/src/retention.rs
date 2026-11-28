//! Retention: after 30 days, idle anonymous matches, the stored command results of idle matches
//! and the boards that have ended are purged.

use std::time::Duration;

use crate::AppState;
use crate::store::{Store, StoreError};

/// How long (30 days) an idle match, its stored command results or an ended board is kept.
pub const RETENTION_SECS: i64 = 2_592_000;

/// How often the purge runs.
pub const PURGE_INTERVAL: Duration = Duration::from_secs(3600);

/// Delete anonymous matches idle for more than [`RETENTION_SECS`] as of `now`, returning
/// their ids. Their events and stored command results go with them (`ON DELETE CASCADE`). A match
/// idle for exactly the retention period is kept.
pub async fn purge_inactive(store: &Store, now: i64) -> Result<Vec<String>, StoreError> {
    store
        .purge_anonymous_idle_before(now - RETENTION_SECS)
        .await
}

/// One purge pass, as of `now`:
///
/// - delete the expired sessions, invites, device codes, reset codes and link codes;
/// - delete the anonymous matches idle for more than [`RETENTION_SECS`] and end their live
///   streams (owned matches are never deleted);
/// - delete the stored command results of every match idle that long (the matches and their
///   events stay);
/// - delete the boards that ended more than [`RETENTION_SECS`] ago, with everything on them.
///
/// Each of the last three logs how many rows it is about to remove, with the cutoff, before it
/// removes them. A failure in one purge is logged and does not stop the others. Returns how many
/// matches were purged; a failed match purge counts as none.
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
    let cutoff = now - RETENTION_SECS;
    let matches = purge_matches(state, now, cutoff).await;
    if let Err(error) = purge_command_results(&state.store, cutoff).await {
        tracing::error!(%error, "command results purge failed");
    }
    if let Err(error) = purge_boards(&state.store, cutoff).await {
        tracing::error!(%error, "board purge failed");
    }
    matches
}

async fn purge_matches(state: &AppState, now: i64, cutoff: i64) -> usize {
    let purged = async {
        let count = state.store.count_anonymous_idle_before(cutoff).await?;
        tracing::info!(count, cutoff, "purging idle anonymous matches");
        purge_inactive(&state.store, now).await
    };
    match purged.await {
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

async fn purge_command_results(store: &Store, cutoff: i64) -> Result<(), StoreError> {
    let count = store.count_command_results_idle_before(cutoff).await?;
    tracing::info!(count, cutoff, "purging command results of idle matches");
    let purged = store.purge_command_results_idle_before(cutoff).await?;
    tracing::info!(purged, "command results purged");
    Ok(())
}

async fn purge_boards(store: &Store, cutoff: i64) -> Result<(), StoreError> {
    let count = store.count_boards_ended_before(cutoff).await?;
    tracing::info!(count, cutoff, "purging ended boards");
    let purged = store.purge_boards_ended_before(cutoff).await?;
    tracing::info!(purged, "ended boards purged");
    Ok(())
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
