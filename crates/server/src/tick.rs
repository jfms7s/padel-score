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
}

/// One tick as of `now`.
pub async fn tick_once(state: &AppState, now: i64) -> TickReport {
    let mut report = TickReport::default();
    match state.store.prune_push_outbox(now - OUTBOX_KEEP_SECS).await {
        Ok(pruned) => report.pruned = pruned,
        Err(error) => tracing::error!(%error, "push outbox prune failed"),
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
        if report.pruned > 0 {
            tracing::info!(pruned = report.pruned, "tick done");
        }
    }
}
