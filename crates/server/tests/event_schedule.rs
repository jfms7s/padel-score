//! Event date, place and sign-up settings, and cancelling an event (Task 12).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::Router;
use axum::http::{Method, StatusCode};
use padel_score_server::AppState;
use padel_score_server::api::now;
use padel_score_server::competitions::non_stop::clock::ClockAction;
use padel_score_server::competitions::tournament::store::Choice;
use padel_score_server::competitions::{Competition, CompetitionChange, NewCompetition, Status};
use padel_score_server::store::StoreError;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Res, app_from, as_session, fast_user, send, state};

const DAY: i64 = 86_400;

fn one_set() -> Value {
    serde_json::to_value(Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    }))
    .unwrap()
}

fn new_event(name: &str) -> Value {
    json!({ "name": name, "kind": "tournament", "format": one_set() })
}

fn new_scheduled(name: &str, starts_at: i64) -> Value {
    let mut body = new_event(name);
    body["schedule"] = json!({
        "starts_at": starts_at, "place": " Club ", "signups": "public",
        "min_teams": 4, "max_teams": 8,
    });
    body
}

struct World {
    state: AppState,
    pool: SqlitePool,
    app: Router,
    owner: String,
    raw: String,
}

async fn world() -> World {
    let (state, pool) = state().await;
    let (owner, raw) = fast_user(&state, "owner@x.test", "Owner").await;
    let app = app_from(&state, 1);
    World {
        state,
        pool,
        app,
        owner,
        raw,
    }
}

impl World {
    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        send(
            &self.app,
            as_session(&self.state, &self.raw, method, path, body),
        )
        .await
    }

    async fn create(&self, body: Value) -> Res {
        self.call(Method::POST, "/api/events", Some(body)).await
    }

    /// Four teams and a court on `id` through the store, so the event can start.
    async fn ready(&self, id: &str) {
        let s = &self.state.store;
        for n in 0..4 {
            let a = s
                .create_or_get_player(&self.owner, &format!("A{n}"), 1)
                .await
                .unwrap()
                .0
                .id;
            let b = s
                .create_or_get_player(&self.owner, &format!("B{n}"), 1)
                .await
                .unwrap()
                .0
                .id;
            s.add_team(&self.owner, id, [a, b], None, 1).await.unwrap();
        }
        s.add_court(&self.owner, id, None).await.unwrap();
    }

    /// A signed-up user on `id`: a row in `event_signups` (the sign-up flow is Task 13).
    async fn sign_up(&self, id: &str, email: &str) -> String {
        let (user, _) = fast_user(&self.state, email, "Joana").await;
        sqlx::query(
            "INSERT INTO event_signups (id, event_id, user1_id, user1_at) VALUES (?, ?, ?, 1)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(id)
        .bind(&user)
        .execute(&self.pool)
        .await
        .unwrap();
        user
    }

    async fn notices(&self, user: &str) -> Vec<Value> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM push_outbox WHERE user_id = ?")
                .bind(user)
                .fetch_all(&self.pool)
                .await
                .unwrap();
        rows.iter()
            .map(|p| serde_json::from_str(p).unwrap())
            .collect()
    }
}

#[tokio::test]
async fn events_without_a_date_work_as_before() {
    let w = world().await;
    let res = w.create(new_event("Cup")).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let schedule = &res.body["event"]["schedule"];
    assert_eq!(schedule["signups"], json!("off"));
    assert_eq!(schedule["starts_at"], Value::Null);
    assert_eq!(schedule["place"], Value::Null);
    assert_eq!(schedule["min_teams"], json!(4));
    assert_eq!(schedule["max_teams"], Value::Null);
    assert_eq!(schedule["deadline_at"], Value::Null);
    assert_eq!(res.body["event"]["cancelled_at"], Value::Null);
    assert_eq!(res.body["join_token"], Value::Null);
}

#[tokio::test]
async fn schedule_round_trips_through_create_and_patch() {
    let w = world().await;
    let start = now() + 3 * DAY;
    let res = w.create(new_scheduled("Cup", start)).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let schedule = &res.body["event"]["schedule"];
    assert_eq!(schedule["starts_at"], json!(start));
    assert_eq!(schedule["place"], json!("Club"));
    assert_eq!(schedule["signups"], json!("public"));
    assert_eq!(schedule["max_teams"], json!(8));
    assert_eq!(schedule["deadline_at"], json!(start - DAY));
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();

    let patch = json!({ "schedule": { "starts_at": start + DAY, "place": null, "max_teams": 16 } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(patch))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let schedule = &res.body["event"]["schedule"];
    assert_eq!(schedule["starts_at"], json!(start + DAY));
    assert_eq!(schedule["place"], Value::Null);
    assert_eq!(schedule["max_teams"], json!(16));
    assert_eq!(schedule["deadline_at"], json!(start));
    // The list carries it too.
    let list = w.call(Method::GET, "/api/events", None).await;
    assert_eq!(list.body["items"][0]["schedule"]["max_teams"], json!(16));

    for (bad, field) in [
        (json!({ "starts_at": now() - 10 }), "starts_at"),
        (json!({ "max_teams": 6 }), "max_teams"),
        (json!({ "deadline_at": start + 9 * DAY }), "deadline_at"),
    ] {
        let res = w
            .call(
                Method::PATCH,
                &format!("/api/events/{id}"),
                Some(json!({ "schedule": bad })),
            )
            .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body["field"], json!(field));
    }
}

#[tokio::test]
async fn turning_signups_on_makes_a_join_token_once() {
    let w = world().await;
    let res = w.create(new_event("Cup")).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    assert_eq!(res.body["join_token"], Value::Null);
    let start = now() + 3 * DAY;
    let on = json!({ "schedule": { "starts_at": start, "max_teams": 8, "signups": "private" } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(on))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let token = res.body["join_token"].as_str().unwrap().to_owned();
    assert!(!token.is_empty());
    // Private to public keeps it, and so does off and on again.
    for signups in ["public", "off", "private"] {
        let res = w
            .call(
                Method::PATCH,
                &format!("/api/events/{id}"),
                Some(json!({ "schedule": { "signups": signups } })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert_eq!(res.body["join_token"], json!(token));
        assert_eq!(res.body["event"]["schedule"]["signups"], json!(signups));
    }
}

#[tokio::test]
async fn signups_cannot_go_off_while_entries_exist() {
    let w = world().await;
    let start = now() + 3 * DAY;
    let res = w.create(new_scheduled("Cup", start)).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    w.sign_up(&id, "joana@x.test").await;
    let off = json!({ "schedule": { "signups": "off" } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(off))
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], json!("signups_exist"));
    let private = json!({ "schedule": { "signups": "private" } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(private))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[tokio::test]
async fn schedule_frozen_after_start() {
    let w = world().await;
    let res = w.create(new_event("Cup")).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    w.ready(&id).await;
    let res = w
        .call(Method::POST, &format!("/api/events/{id}/start"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let patch = json!({ "schedule": { "place": "Club" } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(patch))
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], json!("event_started"));
}

#[tokio::test]
async fn cancel_before_start_reads_cancelled_and_blocks_start() {
    let w = world().await;
    let res = w.create(new_scheduled("Cup", now() + 3 * DAY)).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    w.ready(&id).await;
    let res = w
        .call(Method::POST, &format!("/api/events/{id}/cancel"), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["status"], json!("cancelled"));
    assert!(res.body["event"]["cancelled_at"].is_i64());
    let read = w
        .call(Method::GET, &format!("/api/events/{id}"), None)
        .await;
    assert_eq!(read.body["event"]["status"], json!("cancelled"));
    let list = w.call(Method::GET, "/api/events", None).await;
    assert_eq!(list.body["items"][0]["status"], json!("cancelled"));

    let start = w
        .call(Method::POST, &format!("/api/events/{id}/start"), None)
        .await;
    assert_eq!(start.status, StatusCode::CONFLICT);
    assert_eq!(start.body["error"], json!("event_cancelled"));
    // Cancelling twice is the same answer.
    let again = w
        .call(Method::POST, &format!("/api/events/{id}/cancel"), None)
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["error"], json!("event_cancelled"));
    // Another user's event is a 404.
    let (_, other) = fast_user(&w.state, "other@x.test", "Other").await;
    let res = send(
        &w.app,
        as_session(
            &w.state,
            &other,
            Method::POST,
            &format!("/api/events/{id}/cancel"),
            None,
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancelled_event_can_be_deleted() {
    let w = world().await;
    let res = w.create(new_event("Cup")).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    w.call(Method::POST, &format!("/api/events/{id}/cancel"), None)
        .await;
    let res = w
        .call(Method::DELETE, &format!("/api/events/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.body);
    let res = w
        .call(Method::GET, &format!("/api/events/{id}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_after_start_is_refused() {
    let w = world().await;
    let res = w.create(new_event("Cup")).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    w.ready(&id).await;
    w.call(Method::POST, &format!("/api/events/{id}/start"), None)
        .await;
    let res = w
        .call(Method::POST, &format!("/api/events/{id}/cancel"), None)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], json!("event_started"));
}

#[tokio::test]
async fn cancel_and_changes_tell_the_signed_up_users() {
    let w = world().await;
    let start = now() + 3 * DAY;
    let res = w.create(new_scheduled("Cup", start)).await;
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    let token = res.body["join_token"].as_str().unwrap().to_owned();
    let joana = w.sign_up(&id, "joana@x.test").await;

    // A rename is not a schedule change.
    w.call(
        Method::PATCH,
        &format!("/api/events/{id}"),
        Some(json!({ "name": "Big cup" })),
    )
    .await;
    assert!(w.notices(&joana).await.is_empty());

    let patch = json!({ "schedule": { "place": "Other club" } });
    w.call(Method::PATCH, &format!("/api/events/{id}"), Some(patch))
        .await;
    let got = w.notices(&joana).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["kind"], json!("event_changed"));
    assert_eq!(got[0]["url"], json!(format!("/j/{token}")));
    assert_eq!(got[0]["title"], json!("Big cup"));
    assert_eq!(got[0]["start"], json!(start));
    // The organiser is not told about their own change.
    assert!(w.notices(&w.owner).await.is_empty());

    w.call(Method::POST, &format!("/api/events/{id}/cancel"), None)
        .await;
    let got = w.notices(&joana).await;
    assert_eq!(got.len(), 2);
    assert_eq!(got[1]["kind"], json!("event_cancelled"));
    assert_eq!(got[1]["url"], json!(format!("/j/{token}")));
    assert_eq!(got[1]["start"], json!(start));
}

/// P93: every write on an event refuses a cancelled one, the courts and teams included.
#[tokio::test]
async fn cancelled_event_refuses_every_write() {
    let w = world().await;
    let s = &w.state.store;
    let o = &w.owner;
    let ev: Competition = s
        .create_competition(
            o,
            serde_json::from_value::<NewCompetition>(new_event("Cup")).unwrap(),
            10,
        )
        .await
        .unwrap();
    let id = ev.id.as_str();
    let p1 = s.create_or_get_player(o, "A", 1).await.unwrap().0.id;
    let p2 = s.create_or_get_player(o, "B", 1).await.unwrap().0.id;
    let p3 = s.create_or_get_player(o, "C", 1).await.unwrap().0.id;
    let p4 = s.create_or_get_player(o, "D", 1).await.unwrap().0.id;
    let team = s.add_team(o, id, [p1, p2], None, 1).await.unwrap();
    let court = s.add_court(o, id, None).await.unwrap();
    let cancelled = s.cancel_competition(o, id, 20).await.unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.cancelled_at, Some(20));

    fn refused<T: std::fmt::Debug>(res: Result<T, StoreError>, what: &str) {
        assert!(
            matches!(res, Err(StoreError::Conflict("event_cancelled"))),
            "{what}: {res:?}"
        );
    }
    refused(s.start_competition(o, id, 30).await, "start");
    refused(s.finish_competition(o, id, None, 30).await, "finish");
    refused(s.cancel_competition(o, id, 30).await, "cancel twice");
    refused(
        s.update_competition(
            o,
            id,
            CompetitionChange {
                name: Some("New".into()),
                ..Default::default()
            },
            30,
        )
        .await,
        "rename",
    );
    refused(s.add_team(o, id, [p3, p4], None, 30).await, "add team");
    refused(
        s.update_team(o, id, &team.id, Default::default()).await,
        "update team",
    );
    refused(s.delete_team(o, id, &team.id).await, "delete team");
    refused(s.add_court(o, id, None).await, "add court");
    refused(
        s.rename_court(o, id, &court.id, "Centre".into()).await,
        "rename court",
    );
    refused(s.rotate_court(o, id, &court.id).await, "rotate court");
    refused(s.advance_court(o, id, &court.id).await, "advance court");
    refused(s.delete_court(o, id, &court.id).await, "delete court");
    refused(
        s.create_event_match(o, id, &team.id, "x", None, None, 30)
            .await,
        "create match",
    );
    refused(s.assign_match(o, id, "m", None).await, "assign match");
    refused(
        s.update_event_match(o, id, "m", None, Some("L".into()))
            .await,
        "update match",
    );
    refused(s.redraw_bracket(o, id, 30).await, "redraw");
    refused(s.fill_bracket_courts(o, id).await, "fill courts");
    refused(
        s.decide_bracket(o, id, "m", Choice::Keep, 30).await,
        "decide",
    );
    refused(
        s.clock_action(o, id, ClockAction::Start, 0, 30_000).await,
        "clock",
    );
    refused(
        s.update_competition(
            o,
            id,
            CompetitionChange {
                name: Some("  ".into()),
                ..Default::default()
            },
            30,
        )
        .await,
        "blank rename",
    );
    // Reads and delete stay allowed.
    assert!(s.competition(o, id).await.unwrap().is_some());
    s.delete_competition(o, id).await.unwrap();
}

#[tokio::test]
async fn changing_legs_rechecks_the_maximum() {
    let w = world().await;
    let body = json!({
        "name": "Non-stop", "kind": "non_stop",
        "format": Format::Timed { deuce: DeuceRule::Advantage, draws_allowed: false },
        "settings": { "legs": 1 },
        "schedule": { "starts_at": now() + 3 * DAY, "max_teams": 22, "min_teams": 2, "signups": "private" },
    });
    let res = w.create(body).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    let patch = json!({ "settings": { "legs": 2 } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(patch))
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
    assert_eq!(res.body["field"], json!("max_teams"));
}

#[tokio::test]
async fn null_deadline_over_http_asks_for_the_default() {
    let w = world().await;
    let start = now() + 3 * DAY;
    let mut body = new_scheduled("Cup", start);
    body["schedule"]["deadline_at"] = json!(start - 3600);
    let res = w.create(body).await;
    assert_eq!(
        res.body["event"]["schedule"]["deadline_at"],
        json!(start - 3600)
    );
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    let patch = json!({ "schedule": { "deadline_at": null } });
    let res = w
        .call(Method::PATCH, &format!("/api/events/{id}"), Some(patch))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body["event"]["schedule"]["deadline_at"],
        json!(start - DAY)
    );
}
