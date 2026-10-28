//! The non-stop clock routes (`GET /api/time`, `POST /api/events/{id}/clock`) and the runner
//! that clears practice points at the go and calls time at the end (phase 8, Task 5).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::time::Duration;

use axum::http::{Method, StatusCode, header};
use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, FinalRequest, MatchView, ReplaceRequest,
    apply_command, record_final, replace_from, view_of,
};
use padel_score_server::competitions::non_stop::clock::{COUNT_IN_MS, ClockAction};
use padel_score_server::competitions::{Audience, Kind, NewCompetition, competition_in, kinds};
use padel_score_server::non_stop_api::{end_slot, fire_due, go_slot, run_clock};
use padel_score_server::store::{Store, events_in, match_by_id_in};
use padel_score_server::{AppState, hub::Subscription};
use padel_scoring::{DeuceRule, Event, FinalScore, Format, Status, Team};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Sse, as_device, as_session, device_token, invited_user, req, send, with_admin};

/// A clock tap's time: any fixed unix ms.
const T: i64 = 1_800_000_000_000;
/// The command clock (seconds) for test taps.
const NOW: i64 = 1_000;
/// The default slot length: 15 minutes.
const SLOT_MS: i64 = 900_000;
const GO: i64 = T + COUNT_IN_MS;
const ENDS: i64 = GO + SLOT_MS;

fn timed() -> Format {
    Format::Timed {
        deuce: DeuceRule::GoldenPoint,
        draws_allowed: false,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

struct Ev {
    state: AppState,
    pool: SqlitePool,
    app: axum::Router,
    owner: String,
    raw: String,
    id: String,
    courts: Vec<String>,
}

/// A running non-stop of four teams on two courts: slot 1 is round 1's two matches.
async fn running(settings: Option<Value>) -> Ev {
    let (state, pool, app, owner, raw) = with_admin().await;
    let s = &state.store;
    let ev = s
        .create_competition(
            &owner,
            NewCompetition {
                schedule: None,
                name: "Americano".into(),
                kind: Kind::NonStop,
                format: timed(),
                settings,
            },
            10,
        )
        .await
        .unwrap();
    for [a, b] in [
        ["Ana", "Marta"],
        ["Rui", "Tiago"],
        ["Eva", "Inês"],
        ["Luís", "Nuno"],
    ] {
        let p1 = s.create_or_get_player(&owner, a, 1).await.unwrap().0.id;
        let p2 = s.create_or_get_player(&owner, b, 1).await.unwrap().0.id;
        s.add_team(&owner, &ev.id, [p1, p2], None, 10)
            .await
            .unwrap();
    }
    let mut courts = Vec::new();
    for _ in 0..2 {
        courts.push(s.add_court(&owner, &ev.id, None).await.unwrap().id);
    }
    s.start_competition(&owner, &ev.id, NOW).await.unwrap();
    Ev {
        state,
        pool,
        app,
        owner,
        raw,
        id: ev.id,
        courts,
    }
}

impl Ev {
    fn store(&self) -> &Store {
        &self.state.store
    }

    async fn clock(&self, action: ClockAction, slot: u32, now_ms: i64) {
        self.store()
            .clock_action(&self.owner, &self.id, action, slot, now_ms)
            .await
            .unwrap();
    }

    /// The slot's matches in play order.
    async fn slot(&self, slot: u32) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT n.match_id FROM non_stop_matches n JOIN competition_matches c \
             ON c.match_id = n.match_id WHERE n.event_id = ? AND n.slot = ? ORDER BY c.ordinal",
        )
        .bind(&self.id)
        .bind(slot)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn slot_state(&self, slot: u32) -> String {
        sqlx::query_scalar("SELECT state FROM non_stop_slots WHERE event_id = ? AND slot = ?")
            .bind(&self.id)
            .bind(slot)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// Applied live commands on `match_id`, as the owner.
    async fn tap(&self, match_id: &str, client_id: &str, commands: Vec<Command>) -> MatchView {
        let m = self
            .store()
            .match_owned_by(match_id, &self.owner)
            .await
            .unwrap()
            .unwrap();
        let req = CommandRequest {
            client_id: client_id.into(),
            batch: false,
            base_revision: m.revision,
            commands,
        };
        match apply_command(self.store(), &m, req, NOW, Authority::Owner)
            .await
            .unwrap()
        {
            CommandOutcome::Applied(view) => view,
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    async fn type_result(&self, match_id: &str, a: u16, b: u16) {
        let m = self
            .store()
            .match_owned_by(match_id, &self.owner)
            .await
            .unwrap()
            .unwrap();
        let req = FinalRequest {
            client_id: format!("final:{match_id}:{a}:{b}"),
            base_revision: None,
            score: FinalScore::Timed {
                a,
                b,
                overtime_winner: None,
            },
        };
        match record_final(self.store(), &m, req, Authority::Owner, NOW)
            .await
            .unwrap()
        {
            CommandOutcome::Applied(_) => {}
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    async fn view(&self, match_id: &str) -> MatchView {
        let mut conn = self.pool.acquire().await.unwrap();
        let m = match_by_id_in(&mut conn, match_id).await.unwrap().unwrap();
        let events = events_in(&mut conn, match_id).await.unwrap();
        view_of(&m, &events).unwrap()
    }

    /// Every row of the match's log: (seq, event, state).
    async fn rows(&self, match_id: &str) -> Vec<(i64, Event, String)> {
        let rows: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT seq, event, state FROM events WHERE match_id = ? ORDER BY seq")
                .bind(match_id)
                .fetch_all(&self.pool)
                .await
                .unwrap();
        rows.into_iter()
            .map(|(seq, event, state)| (seq, serde_json::from_str(&event).unwrap(), state))
            .collect()
    }

    async fn time_ups(&self, match_id: &str) -> usize {
        self.rows(match_id)
            .await
            .iter()
            .filter(|(_, e, _)| *e == Event::TimeUp)
            .count()
    }

    async fn detail(&self) -> Value {
        let res = send(
            &self.app,
            as_session(
                &self.state,
                &self.raw,
                Method::GET,
                &format!("/api/events/{}", self.id),
                None,
            ),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body
    }
}

fn points(winners: &[Team]) -> Vec<Command> {
    winners
        .iter()
        .map(|&team| Command::Point { team })
        .collect()
}

/// Golden-point games: four straight points each.
fn games(winners: &[Team]) -> Vec<Command> {
    winners
        .iter()
        .flat_map(|&team| std::iter::repeat_n(Command::Point { team }, 4))
        .collect()
}

fn published(sub: &mut Subscription) -> MatchView {
    let mut last = None;
    while let Ok(view) = sub.try_recv() {
        last = Some(view);
    }
    (*last.expect("a view was published")).clone()
}

fn empty(view: &MatchView) {
    assert!(view.events.is_empty(), "{:?}", view.events);
    assert_eq!(view.score.games, [0, 0]);
}

#[tokio::test]
async fn time_is_public_and_uncached() {
    let (state, _pool, app, _, _) = with_admin().await;
    let _ = state;
    let res = send(&app, req(Method::GET, "/api/time", None, &[])).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let server = res.body["now_ms"].as_i64().unwrap();
    assert!((server - now_ms()).abs() < 5_000, "{server}");
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));
    assert_eq!(res.body.as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn clock_route_rules() {
    let ev = running(None).await;
    let path = format!("/api/events/{}/clock", ev.id);
    let body = json!({ "action": "start", "slot": 1 });

    let anon = send(&ev.app, req(Method::POST, &path, Some(body.clone()), &[])).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED, "{}", anon.body);

    let token = device_token(ev.store(), &ev.owner).await;
    let device = send(
        &ev.app,
        as_device(&token, Method::POST, &path, Some(body.clone())),
    )
    .await;
    assert_eq!(device.status, StatusCode::FORBIDDEN, "{}", device.body);

    let (_b, braw) = invited_user(&ev.state, &ev.app, &ev.raw, "b@x.test", "Rui").await;
    let other = send(
        &ev.app,
        as_session(&ev.state, &braw, Method::POST, &path, Some(body.clone())),
    )
    .await;
    assert_eq!(other.status, StatusCode::NOT_FOUND, "{}", other.body);

    let stale = send(
        &ev.app,
        as_session(
            &ev.state,
            &ev.raw,
            Method::POST,
            &path,
            Some(json!({ "action": "start", "slot": 2 })),
        ),
    )
    .await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.body);
    assert_eq!(stale.body["error"], "not_next_slot");

    let detail = ev.detail().await;
    let court = detail["courts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(ev.courts[0]))
        .unwrap();
    let scorer = court["scorer_token"].as_str().unwrap().to_owned();
    let mut phone = Sse::open(
        &ev.app,
        req(
            Method::GET,
            &format!("/api/court/{scorer}/stream"),
            None,
            &[],
        ),
    )
    .await;
    let first = phone.next("court").await;
    assert_eq!(first["kind_data"]["clock"]["state"], "ready");

    let res = send(
        &ev.app,
        as_session(&ev.state, &ev.raw, Method::POST, &path, Some(body)),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["id"], json!(ev.id));
    assert_eq!(res.body["kind_data"]["clock"]["state"], "running");
    assert_eq!(res.body["kind_data"]["clock"]["slot"], 1);
    assert_eq!(res.header(header::CACHE_CONTROL), Some("no-store"));

    let mut frame = phone.next("court").await;
    while frame["kind_data"]["clock"]["state"] != "running" {
        frame = phone.next("court").await;
    }
    assert_eq!(frame["kind_data"]["clock"]["slot"], 1);
}

#[tokio::test]
async fn go_clears_practice_points() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    let mut before = Vec::new();
    for (i, m) in one.iter().enumerate() {
        ev.tap(m, &format!("pre{i}"), points(&[Team::A; 5])).await;
    }
    ev.clock(ClockAction::Start, 1, T).await;
    for (i, m) in one.iter().enumerate() {
        let v = ev.tap(m, &format!("count{i}"), points(&[Team::B; 3])).await;
        before.push(v.revision);
    }
    let mut subs: Vec<_> = one.iter().map(|m| ev.state.hub.subscribe(m)).collect();

    // Not yet: the count-in is still on.
    assert_eq!(fire_due(&ev.state, GO - 1).await, 0);
    assert_eq!(ev.view(&one[0]).await.events.len(), 8);

    assert_eq!(fire_due(&ev.state, GO).await, 0);
    for ((m, sub), rev) in one.iter().zip(subs.iter_mut()).zip(before) {
        let view = ev.view(m).await;
        empty(&view);
        assert!(view.revision > rev, "{} > {rev}", view.revision);
        let rows = ev.rows(m).await;
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|(_, _, s)| s == "replaced"), "{rows:?}");
        let sent = published(sub);
        assert_eq!(sent, view);
    }
    assert_eq!(ev.slot_state(1).await, "running");
}

#[tokio::test]
async fn resume_does_not_clear_again() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.tap(&one[0], "pre", points(&[Team::A; 2])).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.tap(
        &one[0],
        "real",
        points(&[Team::A, Team::B, Team::A, Team::B]),
    )
    .await;
    ev.clock(ClockAction::Pause, 1, GO + 5_000).await;
    ev.clock(ClockAction::Resume, 1, GO + 20_000).await;
    fire_due(&ev.state, GO + 20_000 + COUNT_IN_MS + 1).await;
    assert_eq!(ev.view(&one[0]).await.events.len(), 4);
}

#[tokio::test]
async fn typed_result_survives_the_go() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    ev.type_result(&one[0], 2, 1).await;
    let typed = ev.view(&one[0]).await;
    fire_due(&ev.state, GO).await;
    let view = ev.view(&one[0]).await;
    assert_eq!(view, typed);
    assert_eq!(view.score.games, [2, 1]);
    assert_eq!(
        view.score.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
}

#[tokio::test]
async fn end_before_go_clears_first() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.tap(&one[0], "pre", points(&[Team::A; 6])).await;
    ev.clock(ClockAction::Start, 1, T).await;
    ev.clock(ClockAction::End, 1, T + 1_000).await;
    assert_eq!(fire_due(&ev.state, T + 2_000).await, 2);
    for m in &one {
        let view = ev.view(m).await;
        assert_eq!(view.events.len(), 1, "{:?}", view.events);
        assert_eq!(view.events[0].event, Event::TimeUp);
        assert!(view.score.overtime);
        assert_eq!(view.score.status, Status::InProgress);
    }
    let rows = ev.rows(&one[0]).await;
    assert_eq!(rows.len(), 7);
    assert!(rows[..6].iter().all(|(_, _, s)| s == "replaced"));
    assert_eq!(ev.slot_state(1).await, "done");
}

#[tokio::test]
async fn fire_due_calls_time_on_the_slot() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    let mut cmds = games(&[Team::A, Team::B, Team::A, Team::B, Team::A]);
    cmds.extend(points(&[Team::A, Team::B, Team::A, Team::A]));
    ev.tap(&one[0], "play", cmds).await;
    let mut subs: Vec<_> = one.iter().map(|m| ev.state.hub.subscribe(m)).collect();

    assert_eq!(fire_due(&ev.state, ENDS - 1).await, 0);
    assert_eq!(fire_due(&ev.state, ENDS).await, 2);

    let won = ev.view(&one[0]).await;
    assert_eq!(won.score.games, [3, 2]);
    assert_eq!(
        won.score.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(published(&mut subs[0]), won);
    let ot = ev.view(&one[1]).await;
    assert!(ot.score.overtime);
    assert_eq!(ot.score.status, Status::InProgress);
    assert_eq!(published(&mut subs[1]), ot);
    assert_eq!(ev.slot_state(1).await, "done");
}

#[tokio::test]
async fn fire_due_twice_applies_once() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.tap(&one[0], "play", games(&[Team::A])).await;
    assert_eq!(fire_due(&ev.state, ENDS).await, 2);
    assert_eq!(fire_due(&ev.state, ENDS).await, 0);
    assert_eq!(fire_due(&ev.state, ENDS + 5_000).await, 0);
    for m in &one {
        assert_eq!(ev.time_ups(m).await, 1);
    }
}

#[tokio::test]
async fn typed_result_before_time_up_is_kept() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.type_result(&one[0], 4, 2).await;
    let rows = ev.rows(&one[0]).await.len();
    assert_eq!(fire_due(&ev.state, ENDS).await, 1);
    let view = ev.view(&one[0]).await;
    assert_eq!(view.score.games, [4, 2]);
    assert_eq!(ev.rows(&one[0]).await.len(), rows);
}

#[tokio::test]
async fn overdue_slot_fires_on_first_pass() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.tap(&one[0], "pre", points(&[Team::B; 3])).await;
    ev.clock(ClockAction::Start, 1, T).await;
    // A restart: a fresh state on the same database.
    let fresh = AppState::new(ev.state.config.clone(), Store::new(ev.pool.clone()));
    assert_eq!(fire_due(&fresh, ENDS + 60_000).await, 2);
    let rows = ev.rows(&one[0]).await;
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[..3].iter().all(|(_, _, s)| s == "replaced"));
    assert_eq!(rows[3].1, Event::TimeUp);
    assert_eq!(rows[3].2, "effective");
    assert_eq!(ev.time_ups(&one[0]).await, 1);
    let view = ev.view(&one[0]).await;
    assert_eq!(view.events.len(), 1);
    assert_eq!(view.score.games, [0, 0]);
}

#[tokio::test]
async fn paused_slot_survives_restart() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.tap(&one[0], "pre", points(&[Team::B; 3])).await;
    ev.clock(ClockAction::Start, 1, T).await;
    ev.clock(ClockAction::Pause, 1, T + 1_000).await;
    let fresh = AppState::new(ev.state.config.clone(), Store::new(ev.pool.clone()));
    assert_eq!(fire_due(&fresh, T + 100 * SLOT_MS).await, 0);
    assert_eq!(ev.slot_state(1).await, "paused");
    assert_eq!(ev.view(&one[0]).await.events.len(), 3);
    for m in &one {
        assert_eq!(ev.time_ups(m).await, 0);
    }
}

#[tokio::test]
async fn replace_dropping_time_up_is_reasserted() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.tap(&one[1], "play", points(&[Team::A])).await;
    assert_eq!(fire_due(&ev.state, ENDS).await, 2);
    let after = ev.view(&one[1]).await;
    let time_up_seq = after.events.last().unwrap().seq;
    assert_eq!(after.events.last().unwrap().event, Event::TimeUp);

    // An offline phone's "keep mine" from before the time-up.
    let m = ev
        .store()
        .match_owned_by(&one[1], &ev.owner)
        .await
        .unwrap()
        .unwrap();
    let req = ReplaceRequest {
        client_id: "keep-mine".into(),
        base_revision: m.revision,
        base_seq: time_up_seq - 1,
        commands: points(&[Team::B, Team::B]),
    };
    match replace_from(ev.store(), &m, req, NOW, Authority::Scorer)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(view) => {
            assert!(view.events.iter().all(|e| e.event != Event::TimeUp));
        }
        other => panic!("expected Applied, got {other:?}"),
    }
    let flagged: i64 =
        sqlx::query_scalar("SELECT needs_time_up FROM non_stop_matches WHERE match_id = ?")
            .bind(&one[1])
            .fetch_one(&ev.pool)
            .await
            .unwrap();
    assert_eq!(flagged, 1);

    assert_eq!(fire_due(&ev.state, ENDS + 1_000).await, 1);
    let view = ev.view(&one[1]).await;
    let kinds: Vec<Event> = view.events.iter().map(|e| e.event).collect();
    assert_eq!(
        kinds,
        vec![
            Event::Point(Team::A),
            Event::Point(Team::B),
            Event::Point(Team::B),
            Event::TimeUp
        ]
    );
    assert!(view.events.last().unwrap().seq > time_up_seq);
    assert_eq!(fire_due(&ev.state, ENDS + 2_000).await, 0);
}

#[tokio::test]
async fn undo_of_time_up_is_reasserted() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.tap(&one[1], "play", points(&[Team::A])).await;
    assert_eq!(fire_due(&ev.state, ENDS).await, 2);
    let time_up_seq = ev.view(&one[1]).await.events.last().unwrap().seq;

    // The scorer undoes the time-up.
    let m = ev
        .store()
        .match_owned_by(&one[1], &ev.owner)
        .await
        .unwrap()
        .unwrap();
    let req = CommandRequest {
        client_id: "undo-time-up".into(),
        batch: false,
        base_revision: m.revision,
        commands: vec![Command::Undo {
            target_seq: time_up_seq,
        }],
    };
    match apply_command(ev.store(), &m, req, NOW, Authority::Scorer)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(view) => {
            assert!(view.events.iter().all(|e| e.event != Event::TimeUp));
        }
        other => panic!("expected Applied, got {other:?}"),
    }
    let flagged: i64 =
        sqlx::query_scalar("SELECT needs_time_up FROM non_stop_matches WHERE match_id = ?")
            .bind(&one[1])
            .fetch_one(&ev.pool)
            .await
            .unwrap();
    assert_eq!(flagged, 1);

    assert_eq!(fire_due(&ev.state, ENDS + 1_000).await, 1);
    let view = ev.view(&one[1]).await;
    let kinds: Vec<Event> = view.events.iter().map(|e| e.event).collect();
    assert_eq!(kinds, vec![Event::Point(Team::A), Event::TimeUp]);
    assert!(view.events.last().unwrap().seq > time_up_seq);
    let flagged: i64 =
        sqlx::query_scalar("SELECT needs_time_up FROM non_stop_matches WHERE match_id = ?")
            .bind(&one[1])
            .fetch_one(&ev.pool)
            .await
            .unwrap();
    assert_eq!(flagged, 0);
    assert_eq!(fire_due(&ev.state, ENDS + 2_000).await, 0);
}

#[tokio::test]
async fn clock_went_turns_true_at_the_go() {
    let ev = running(None).await;
    let went = |ev: &Ev| {
        let pool = ev.pool.clone();
        let (owner, id) = (ev.owner.clone(), ev.id.clone());
        async move {
            let mut conn = pool.acquire().await.unwrap();
            let comp = competition_in(&mut conn, &owner, &id)
                .await
                .unwrap()
                .unwrap();
            let kd = kinds::kind_data(Kind::NonStop, &mut conn, &comp, Audience::Coordinator)
                .await
                .unwrap()
                .unwrap();
            kd["clock"]["went"].clone()
        }
    };
    assert_eq!(went(&ev).await, json!(false));
    ev.clock(ClockAction::Start, 1, T).await;
    assert_eq!(went(&ev).await, json!(false));
    fire_due(&ev.state, GO).await;
    assert_eq!(went(&ev).await, json!(true));
}

#[tokio::test]
async fn go_skips_a_slot_paused_since_it_was_found_due() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.tap(&one[0], "pre", points(&[Team::A; 3])).await;
    ev.clock(ClockAction::Start, 1, T).await;
    // The runner found the slot due at GO; a Pause in the count-in commits before its go runs.
    ev.clock(ClockAction::Pause, 1, T + 1_000).await;
    let revision = ev.view(&one[0]).await.revision;
    let went = go_slot(ev.store(), &ev.id, 1, GO, GO / 1000).await.unwrap();
    assert!(went.is_none());
    let view = ev.view(&one[0]).await;
    assert_eq!(view.events.len(), 3);
    assert_eq!(view.revision, revision);
    let started: Option<i64> = sqlx::query_scalar(
        "SELECT started_at_ms FROM non_stop_slots WHERE event_id = ? AND slot = 1",
    )
    .bind(&ev.id)
    .fetch_one(&ev.pool)
    .await
    .unwrap();
    assert_eq!(started, None);
    assert_eq!(ev.slot_state(1).await, "paused");

    // Still due: the go runs.
    ev.clock(ClockAction::Resume, 1, T + 2_000).await;
    let went = go_slot(ev.store(), &ev.id, 1, T + 2_000 + COUNT_IN_MS, 0)
        .await
        .unwrap();
    assert_eq!(went.unwrap().0.len(), 2);
    assert!(ev.view(&one[0]).await.events.is_empty());
}

#[tokio::test]
async fn end_now_calls_time_at_once() {
    let ev = running(None).await;
    let one = ev.slot(1).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    ev.clock(ClockAction::End, 1, GO + 60_000).await;
    assert_eq!(fire_due(&ev.state, GO + 60_000).await, 2);
    for m in &one {
        assert_eq!(ev.time_ups(m).await, 1);
    }
}

#[tokio::test]
async fn run_clock_wakes_on_start() {
    let ev = running(Some(json!({ "match_minutes": 1 }))).await;
    let one = ev.slot(1).await;
    let runner = tokio::spawn(run_clock(ev.state.clone()));
    let res = send(
        &ev.app,
        as_session(
            &ev.state,
            &ev.raw,
            Method::POST,
            &format!("/api/events/{}/clock", ev.id),
            Some(json!({ "action": "start", "slot": 1 })),
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let past = now_ms() - 1_000;
    sqlx::query(
        "UPDATE non_stop_slots SET starts_at_ms = ?, ends_at_ms = ? WHERE event_id = ? AND slot = 1",
    )
    .bind(past - 60_000)
    .bind(past)
    .bind(&ev.id)
    .execute(&ev.pool)
    .await
    .unwrap();
    ev.state.clock_wake.notify_one();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if ev.time_ups(&one[0]).await == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no time-up within 2 s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ev.state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), runner)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn end_skips_a_slot_paused_or_ended_since_it_was_found_due() {
    let ev = running(None).await;
    ev.clock(ClockAction::Start, 1, T).await;
    fire_due(&ev.state, GO).await;
    let done_at = || async {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT done_at_ms FROM non_stop_slots WHERE event_id = ? AND slot = 1",
        )
        .bind(&ev.id)
        .fetch_one(&ev.pool)
        .await
        .unwrap()
    };
    // The runner found the slot due at ENDS; a Pause committed before its end runs.
    ev.clock(ClockAction::Pause, 1, ENDS - 1_000).await;
    assert!(!end_slot(ev.store(), &ev.id, 1, ENDS).await.unwrap());
    assert_eq!(ev.slot_state(1).await, "paused");
    assert_eq!(done_at().await, None);

    // Resumed with a second left: not due at ENDS + 500, due at ENDS + 1_000 + COUNT_IN_MS.
    ev.clock(ClockAction::Resume, 1, ENDS).await;
    assert!(!end_slot(ev.store(), &ev.id, 1, ENDS + 500).await.unwrap());
    assert_eq!(ev.slot_state(1).await, "running");
    let end = ENDS + 1_000 + COUNT_IN_MS;
    assert!(end_slot(ev.store(), &ev.id, 1, end).await.unwrap());
    assert_eq!(ev.slot_state(1).await, "done");
    assert_eq!(done_at().await, Some(end));

    // Once done (here by the runner, as by an End), a late end writes nothing.
    assert!(!end_slot(ev.store(), &ev.id, 1, end + 5_000).await.unwrap());
    assert_eq!(done_at().await, Some(end));
}
