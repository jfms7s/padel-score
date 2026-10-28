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

#[tokio::test]
async fn tick_runs_deadlines() {
    use padel_score_server::competitions::schedule::{ScheduleInput, Signups};
    use padel_score_server::competitions::signups::JoinAs;
    use padel_score_server::competitions::{Kind, NewCompetition};
    use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
    const T0: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;
    let (state, pool, _app, admin, _raw) = with_admin().await;
    let ev = state
        .store
        .create_competition(
            &admin,
            NewCompetition {
                name: "Friday cup".into(),
                kind: Kind::Tournament,
                format: Format::Sets(SetRules {
                    deuce: DeuceRule::Advantage,
                    sets: SetsFormat::One,
                    deciding: DecidingSet::FullSet,
                }),
                settings: None,
                schedule: Some(ScheduleInput {
                    starts_at: Some(Some(T0 + 3 * DAY)),
                    signups: Some(Signups::Public),
                    max_teams: Some(Some(8)),
                    ..Default::default()
                }),
            },
            T0,
        )
        .await
        .unwrap();
    let rui = state
        .store
        .create_user(
            padel_score_server::accounts_store::NewUser {
                email: "rui@x.test".into(),
                display_name: "Rui".into(),
                password_hash: "x".into(),
            },
            T0,
        )
        .await
        .unwrap()
        .id;
    let token = ev.join_token.clone().unwrap();
    state
        .store
        .join_event(&token, &rui, JoinAs::Solo, T0 + 10)
        .await
        .unwrap();
    // Before the deadline nothing happens.
    assert_eq!(tick_once(&state, T0 + DAY).await.closed_signups, 0);
    // At the deadline one team is short of the minimum: the event is cancelled and Rui told.
    let report = tick_once(&state, T0 + 2 * DAY).await;
    assert_eq!(report.closed_signups, 1);
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT payload FROM push_outbox WHERE user_id = ?")
            .bind(&rui)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(kinds.len(), 1);
    assert!(kinds[0].contains("event_cancelled"), "{}", kinds[0]);
    // The organiser is told too, and the desk's stream was nudged.
    let own: Vec<String> = sqlx::query_scalar("SELECT payload FROM push_outbox WHERE user_id = ?")
        .bind(&admin)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(own.len(), 1);
    assert!(own[0].contains("event_cancelled"), "{}", own[0]);
    assert_eq!(tick_once(&state, T0 + 2 * DAY + 60).await.closed_signups, 0);
}

mod reminders {
    use padel_score_server::AppState;
    use padel_score_server::accounts_store::NewUser;
    use padel_score_server::boards::{NewBoard, WEEK_SECS};
    use padel_score_server::competitions::schedule::{ScheduleInput, Signups};
    use padel_score_server::competitions::signups::JoinAs;
    use padel_score_server::competitions::{Kind, NewCompetition};
    use padel_score_server::scheduled::{Listing, NewScheduled};
    use padel_score_server::tick::tick_once;
    use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
    use serde_json::Value;
    use sqlx::SqlitePool;

    use super::with_admin;

    const T0: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;
    const HOUR: i64 = 3600;
    const START: i64 = T0 + DAY + 6 * HOUR;

    async fn user(state: &AppState, name: &str) -> String {
        state
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
            .unwrap()
            .id
    }

    fn one_set() -> Format {
        Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::One,
            deciding: DecidingSet::FullSet,
        })
    }

    /// The `(user, payload)` rows of `kind`, by user.
    async fn rows(pool: &SqlitePool, kind: &str) -> Vec<(String, Value)> {
        let all: Vec<(String, String)> =
            sqlx::query_as("SELECT user_id, payload FROM push_outbox ORDER BY user_id, id")
                .fetch_all(pool)
                .await
                .unwrap();
        all.into_iter()
            .map(|(u, p)| (u, serde_json::from_str::<Value>(&p).unwrap()))
            .filter(|(_, p)| p["kind"] == kind)
            .collect()
    }

    fn users(rows: &[(String, Value)]) -> Vec<String> {
        let mut users: Vec<String> = rows.iter().map(|(u, _)| u.clone()).collect();
        users.sort();
        users
    }

    /// A board with two overlapping games starting at `START` and half an hour later; the
    /// first loses its fourth player, who stays on the board, and a fifth member has no time.
    /// (token, first game's three, the leaver, the idle member, the second game's four)
    async fn short_world(state: &AppState) -> (String, Vec<String>, String, String, Vec<String>) {
        let mut first = Vec::new();
        let mut second = Vec::new();
        for i in 0..4 {
            first.push(user(state, &format!("Aa{i}")).await);
            second.push(user(state, &format!("Bb{i}")).await);
        }
        let idle = user(state, "Idle").await;
        let token = state
            .store
            .create_board(
                &first[0],
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
        for (i, who) in first.iter().enumerate() {
            let at = T0 + i64::try_from(i).unwrap();
            state
                .store
                .add_free(&token, who, START, START + HOUR, at)
                .await
                .unwrap();
        }
        for (i, who) in second.iter().enumerate() {
            let at = T0 + 10 + i64::try_from(i).unwrap();
            state
                .store
                .add_free(&token, who, START + HOUR / 2, START + HOUR / 2 + HOUR, at)
                .await
                .unwrap();
        }
        state
            .store
            .join_board(&token, &idle, T0 + 20)
            .await
            .unwrap();
        let view = state
            .store
            .board_view(&token, Some(&first[0]), T0 + 30)
            .await
            .unwrap()
            .unwrap();
        let game = &view.games[0];
        let leaver = first.pop().unwrap();
        state
            .store
            .leave_game(&token, &game.id, &leaver, game.version, T0 + 40)
            .await
            .unwrap();
        (token, first, leaver, idle, second)
    }

    #[tokio::test]
    async fn short_game_alert_goes_once_to_free_members() {
        let (state, pool, _app, _admin, _raw) = with_admin().await;
        let (_token, _first, leaver, idle, _second) = short_world(&state).await;
        let report = tick_once(&state, START - 3 * HOUR).await;
        assert_eq!(report.reminders, 2);
        let rows = rows(&pool, "short_game").await;
        // The leaver and the idle member are free; the three in the game and the four in the
        // overlapping game are not.
        let mut free = vec![leaver, idle];
        free.sort();
        assert_eq!(users(&rows), free);
        let payload = &rows[0].1;
        assert_eq!(payload["title"], "Thursday padel");
        assert_eq!(payload["start"], START);
        assert_eq!(payload["end"], START + HOUR);
        assert!(payload["url"].as_str().unwrap().starts_with("/b/"));
        // A second tick, and a later one, queue nothing more.
        assert_eq!(tick_once(&state, START - 3 * HOUR + 60).await.reminders, 0);
        assert_eq!(tick_once(&state, START - 2 * HOUR).await.reminders, 0);
        assert_eq!(super::reminders::rows(&pool, "short_game").await.len(), 2);
    }

    #[tokio::test]
    async fn nothing_for_games_more_than_24h_away() {
        let (state, pool, _app, _admin, _raw) = with_admin().await;
        short_world(&state).await;
        // START is 30 hours after this tick.
        let early = START - 30 * HOUR;
        assert_eq!(tick_once(&state, early).await.reminders, 0);
        assert!(rows(&pool, "short_game").await.is_empty());
        // Exactly 24h away is inside the window; a started game is not.
        assert_eq!(tick_once(&state, START - DAY).await.reminders, 2);
        let quiet = short_world_started().await;
        assert_eq!(quiet, 0);
    }

    /// A short game that has started: no reminder.
    async fn short_world_started() -> usize {
        let (state, _pool, _app, _admin, _raw) = with_admin().await;
        short_world(&state).await;
        tick_once(&state, START).await.reminders
    }

    #[tokio::test]
    async fn event_soon_goes_once() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        let (rui, ana, outsider) = (
            user(&state, "Rui").await,
            user(&state, "Ana").await,
            user(&state, "Marta").await,
        );
        let ev = state
            .store
            .create_competition(
                &admin,
                NewCompetition {
                    name: "Friday cup".into(),
                    kind: Kind::Tournament,
                    format: one_set(),
                    settings: None,
                    schedule: Some(ScheduleInput {
                        starts_at: Some(Some(START)),
                        signups: Some(Signups::Public),
                        max_teams: Some(Some(8)),
                        deadline_at: Some(Some(START - HOUR)),
                        ..Default::default()
                    }),
                },
                T0,
            )
            .await
            .unwrap();
        let token = ev.join_token.clone().unwrap();
        state
            .store
            .join_event(&token, &rui, JoinAs::Solo, T0 + 1)
            .await
            .unwrap();
        state
            .store
            .join_event(&token, &ana, JoinAs::Solo, T0 + 2)
            .await
            .unwrap();
        // Too early: 30 hours before the start.
        assert_eq!(tick_once(&state, START - 30 * HOUR).await.reminders, 0);
        // Inside the day, before the deadline (which is 24h before the start).
        let report = tick_once(&state, START - 24 * HOUR + 1).await;
        assert_eq!(report.reminders, 2);
        let rows = rows(&pool, "event_soon").await;
        let mut both = vec![rui.clone(), ana.clone()];
        both.sort();
        assert_eq!(users(&rows), both);
        assert!(!users(&rows).contains(&outsider));
        assert_eq!(rows[0].1["title"], "Friday cup");
        assert_eq!(rows[0].1["start"], START);
        assert_eq!(rows[0].1["url"], format!("/j/{token}"));
        assert_eq!(tick_once(&state, START - 23 * HOUR).await.reminders, 0);
        assert_eq!(rows_len(&pool).await, 2);
    }

    async fn rows_len(pool: &SqlitePool) -> usize {
        rows(pool, "event_soon").await.len()
    }

    #[tokio::test]
    async fn match_short_goes_to_players() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        let (a, b, c, d) = (
            user(&state, "Ana").await,
            user(&state, "Rui").await,
            user(&state, "Marta").await,
            user(&state, "Joana").await,
        );
        let new = |start: i64| NewScheduled {
            starts_at: start,
            place: None,
            listing: Listing::Private,
            format: one_set(),
            places: Default::default(),
        };
        let three = state
            .store
            .create_scheduled(&admin, new(START), T0)
            .await
            .unwrap();
        let four = state
            .store
            .create_scheduled(&admin, new(START + HOUR), T0)
            .await
            .unwrap();
        let two = state
            .store
            .create_scheduled(&admin, new(START + 2 * HOUR), T0)
            .await
            .unwrap();
        for (m, who) in [
            (&three, vec![&a, &b, &c]),
            (&four, vec![&a, &b, &c, &d]),
            (&two, vec![&a, &b]),
        ] {
            for u in who {
                state
                    .store
                    .join_scheduled(&m.join_token, u, T0 + 1)
                    .await
                    .unwrap();
            }
        }
        let report = tick_once(&state, START - 2 * HOUR).await;
        assert_eq!(report.reminders, 3);
        let rows = rows(&pool, "match_short").await;
        let mut players = vec![a.clone(), b.clone(), c.clone()];
        players.sort();
        assert_eq!(users(&rows), players);
        assert_eq!(rows[0].1["url"], format!("/j/{}", three.join_token));
        assert_eq!(rows[0].1["start"], START);
        assert_eq!(rows[0].1["name"], "Ana");
        assert_eq!(tick_once(&state, START - HOUR).await.reminders, 0);
    }

    /// A public tournament with sign-ups open until an hour before the start.
    async fn event(state: &AppState, owner: &str) -> (String, String) {
        let ev = state
            .store
            .create_competition(
                owner,
                NewCompetition {
                    name: "Friday cup".into(),
                    kind: Kind::Tournament,
                    format: one_set(),
                    settings: None,
                    schedule: Some(ScheduleInput {
                        starts_at: Some(Some(START)),
                        signups: Some(Signups::Public),
                        max_teams: Some(Some(8)),
                        deadline_at: Some(Some(START - HOUR)),
                        ..Default::default()
                    }),
                },
                T0,
            )
            .await
            .unwrap();
        (ev.id, ev.join_token.unwrap())
    }

    #[tokio::test]
    async fn event_soon_reaches_late_signups_and_a_rescheduled_event() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        let (id, token) = event(&state, &admin).await;
        let mut first = Vec::new();
        for n in ["Rui", "Ana2"] {
            let u = user(&state, n).await;
            state
                .store
                .join_event(&token, &u, JoinAs::Solo, T0 + 1)
                .await
                .unwrap();
            first.push(u);
        }
        let inside = START - 20 * HOUR;
        assert_eq!(tick_once(&state, inside).await.reminders, 2);
        // A pair signing up inside the last day is told too; a lone solo is not in the event.
        for n in ["Marta", "Joana", "Solo"] {
            let u = user(&state, n).await;
            state
                .store
                .join_event(&token, &u, JoinAs::Solo, inside + 1)
                .await
                .unwrap();
        }
        assert_eq!(tick_once(&state, inside + 60).await.reminders, 2);
        assert_eq!(rows(&pool, "event_soon").await.len(), 4);
        assert_eq!(tick_once(&state, inside + 120).await.reminders, 0);
        // Moved by an hour: everyone in is told again.
        sqlx::query("UPDATE competitions SET starts_at = starts_at + ? WHERE id = ?")
            .bind(HOUR)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(tick_once(&state, inside + 180).await.reminders, 4);
    }

    #[tokio::test]
    async fn event_soon_not_for_cancelled_or_started_events() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        let (id, token) = event(&state, &admin).await;
        for n in ["Rui", "Ana2"] {
            let u = user(&state, n).await;
            state
                .store
                .join_event(&token, &u, JoinAs::Solo, T0 + 1)
                .await
                .unwrap();
        }
        sqlx::query("UPDATE competitions SET cancelled_at = ? WHERE id = ?")
            .bind(T0 + 2)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(tick_once(&state, START - 20 * HOUR).await.reminders, 0);
        sqlx::query("UPDATE competitions SET cancelled_at = NULL, status = 'running' WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(tick_once(&state, START - 20 * HOUR).await.reminders, 0);
    }

    #[tokio::test]
    async fn a_moved_game_or_match_reminds_again() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        short_world(&state).await;
        let (a, b, c) = (
            user(&state, "P1").await,
            user(&state, "P2").await,
            user(&state, "P3").await,
        );
        let m = state
            .store
            .create_scheduled(
                &admin,
                NewScheduled {
                    starts_at: START + 5 * HOUR,
                    place: None,
                    listing: Listing::Private,
                    format: one_set(),
                    places: Default::default(),
                },
                T0,
            )
            .await
            .unwrap();
        for u in [&a, &b, &c] {
            state
                .store
                .join_scheduled(&m.join_token, u, T0 + 1)
                .await
                .unwrap();
        }
        let now = START - 10 * HOUR;
        assert_eq!(tick_once(&state, now).await.reminders, 5);
        assert_eq!(tick_once(&state, now + 60).await.reminders, 0);
        sqlx::query(
            "UPDATE board_games SET starts_at = starts_at + 3600, ends_at = ends_at + 3600",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE scheduled_matches SET starts_at = starts_at + 3600")
            .execute(&pool)
            .await
            .unwrap();
        // The two free members and the three players are told again for the new start.
        assert_eq!(tick_once(&state, now + 120).await.reminders, 5);
        assert_eq!(rows(&pool, "short_game").await.len(), 4);
        assert_eq!(rows(&pool, "match_short").await.len(), 6);
    }

    #[tokio::test]
    async fn nothing_for_cancelled_or_started_matches_or_played_games() {
        let (state, pool, _app, admin, _raw) = with_admin().await;
        short_world(&state).await;
        let mut tokens = Vec::new();
        for i in 0..2 {
            let m = state
                .store
                .create_scheduled(
                    &admin,
                    NewScheduled {
                        starts_at: START + 5 * HOUR + i * HOUR,
                        place: None,
                        listing: Listing::Private,
                        format: one_set(),
                        places: Default::default(),
                    },
                    T0,
                )
                .await
                .unwrap();
            for n in 0..3 {
                let u = user(&state, &format!("P{i}{n}")).await;
                state
                    .store
                    .join_scheduled(&m.join_token, &u, T0 + 1)
                    .await
                    .unwrap();
            }
            tokens.push(m.id);
        }
        sqlx::query("INSERT INTO matches (id, scorer_token, spectator_token, format, start, teams, created_at, last_activity_at) VALUES ('m1', 's', 'p', '{}', '{}', '{}', 1, 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE scheduled_matches SET cancelled_at = 1 WHERE id = ?")
            .bind(&tokens[0])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE scheduled_matches SET started_at = 1, match_id = 'm1' WHERE id = ?")
            .bind(&tokens[1])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE board_games SET match_id = 'm1', played_at = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(tick_once(&state, START - 10 * HOUR).await.reminders, 0);
    }
}
