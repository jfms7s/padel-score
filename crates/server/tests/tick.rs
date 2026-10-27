#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! The minute tick: housekeeping that later jobs join.

mod support;

use padel_score_server::push::{Notice, NoticeKind, OUTBOX_KEEP_SECS, Payload};
use padel_score_server::tick::tick_once;
use support::with_admin;

#[tokio::test]
async fn tick_prunes_old_outbox_rows() {
    let (state, pool, _app, admin, _raw) = with_admin().await;
    let now = 10_000_000;
    let notice = |dedupe: &str| Notice {
        user_id: admin.clone(),
        payload: Payload {
            kind: NoticeKind::Test,
            url: "/account".to_owned(),
            ..Payload::default()
        },
        dedupe: Some(dedupe.to_owned()),
    };
    state
        .store
        .queue(&[notice("old")], now - OUTBOX_KEEP_SECS - 1)
        .await
        .unwrap();
    state
        .store
        .queue(&[notice("recent")], now - OUTBOX_KEEP_SECS + 1)
        .await
        .unwrap();
    // A sent row past the window goes too.
    state
        .store
        .queue(&[notice("old-sent")], now - OUTBOX_KEEP_SECS - 1)
        .await
        .unwrap();
    sqlx::query("UPDATE push_outbox SET sent_at = ? WHERE dedupe = 'old-sent'")
        .bind(now - OUTBOX_KEEP_SECS)
        .execute(&pool)
        .await
        .unwrap();
    let report = tick_once(&state, now).await;
    assert_eq!(report.pruned, 2);
    let left: Vec<String> = sqlx::query_scalar("SELECT dedupe FROM push_outbox")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["recent"]);
}
