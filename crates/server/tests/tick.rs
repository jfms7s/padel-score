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

#[tokio::test]
async fn tick_expires_requests_of_started_games() {
    use padel_score_server::accounts_store::NewUser;
    use padel_score_server::boards::{NewBoard, WEEK_SECS};
    const T0: i64 = 1_800_000_000;
    const HOUR: i64 = 3600;
    const START: i64 = T0 + 86_400;
    let (state, pool, _app, _admin, _raw) = with_admin().await;
    let mut who = Vec::new();
    for name in ["Ana", "Rui", "Marta", "Joana"] {
        let user = state
            .store
            .create_user(
                NewUser {
                    email: format!("{}@x.test", name.to_lowercase()),
                    display_name: name.to_owned(),
                    password_hash: "x".to_owned(),
                },
                1,
            )
            .await
            .unwrap();
        who.push(user.id);
    }
    let token = state
        .store
        .create_board(
            &who[0],
            NewBoard {
                title: "Thursday padel".to_owned(),
                place: None,
                time_zone: "Europe/Lisbon".to_owned(),
                starts_at: START,
                ends_at: START + WEEK_SECS,
            },
            T0,
        )
        .await
        .unwrap()
        .board
        .token;
    // Three hours free for all four make two back-to-back games of 90 minutes.
    for (i, id) in who.iter().enumerate() {
        state
            .store
            .add_free(
                &token,
                id,
                START,
                START + 3 * HOUR,
                T0 + i64::try_from(i).unwrap(),
            )
            .await
            .unwrap();
    }
    let view = state
        .store
        .board_view(&token, Some(&who[0]), T0 + 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.games.len(), 2);
    let mut ids = Vec::new();
    for game in &view.games {
        let asked = state
            .store
            .ask_time(
                &token,
                &game.id,
                &who[0],
                game.version,
                START + 4 * HOUR,
                START + 5 * HOUR,
                false,
                T0 + 20,
            )
            .await
            .unwrap();
        ids.push(asked.requests.last().unwrap().id.clone());
    }
    // Half an hour in, the first game has started and the second has not.
    let report = tick_once(&state, START + 1800).await;
    assert_eq!(report.expired_requests, 1);
    let status = |id: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT status FROM board_requests WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let first = ids.first().unwrap().clone();
    let second = ids.get(1).unwrap().clone();
    assert_eq!(status(first).await, "expired");
    assert_eq!(status(second).await, "pending");
    assert_eq!(tick_once(&state, START + 1800).await.expired_requests, 0);
}
