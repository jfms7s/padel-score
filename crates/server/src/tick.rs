//! The minute tick: housekeeping and timed jobs. Each job logs its own error and the tick goes
//! on to the next.

use std::time::Duration;

use crate::AppState;
use crate::push::OUTBOX_KEEP_SECS;

pub const TICK: Duration = Duration::from_secs(60);

#[derive(Default, Debug)]
pub struct TickReport {
    /// Outbox rows deleted for being older than [`OUTBOX_KEEP_SECS`].
    pub pruned: usize,
    /// Board swap and time requests set to `expired` because a game changed or started.
    pub expired_requests: usize,
    /// Events whose sign-up deadline ran: closed, or cancelled for too few teams.
    pub closed_signups: usize,
}

/// One tick as of `now`.
pub async fn tick_once(state: &AppState, now: i64) -> TickReport {
    let mut report = TickReport::default();
    match state.store.prune_push_outbox(now - OUTBOX_KEEP_SECS).await {
        Ok(pruned) => report.pruned = pruned,
        Err(error) => tracing::error!(%error, "push outbox prune failed"),
    }
    match state.store.expire_requests(now).await {
        Ok(expired) => report.expired_requests = expired,
        Err(error) => tracing::error!(%error, "board request expiry failed"),
    }
    match state.store.close_due_signups(now).await {
        Ok(closed) => report.closed_signups = closed,
        Err(error) => tracing::error!(%error, "sign-up deadline job failed"),
    }
    if report.closed_signups > 0 {
        // A cancelled event queued notices.
        state.push.wake();
    }
    report
}

/// Tick every [`TICK`] until the shutdown token fires. The first tick runs at once.
pub async fn run(state: AppState) {
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = state.shutdown.cancelled() => return,
            _ = ticker.tick() => {}
        }
        let report = tick_once(&state, crate::api::now()).await;
        if report.pruned + report.expired_requests + report.closed_signups > 0 {
            tracing::info!(
                pruned = report.pruned,
                expired_requests = report.expired_requests,
                closed_signups = report.closed_signups,
                "tick done"
            );
        }
    }
}
