//! Scheduled matches: Play later, the places, the line, the 1h cutoff and Start (Task 16).
//! Store tests use an explicit clock from `T0`; the API tests the real one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::http::{Method, StatusCode};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::api::now;
use padel_score_server::db;
use padel_score_server::scheduled::{
    Listing, NewScheduled, ScheduledChange, ScheduledState, ScheduledView,
};
use padel_score_server::starting::{StartedFrom, create_match_from};
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{app_from, as_session, fast_user, req, send, state};

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;
const HOUR: i64 = 3600;
const START: i64 = T0 + DAY;

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

async fn user(s: &Store, name: &str) -> String {
    s.create_user(
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

fn new_at(starts_at: i64) -> NewScheduled {
    NewScheduled {
        starts_at,
        place: Some("Club court 2".to_owned()),
        listing: Listing::Private,
        format: one_set(),
        places: Default::default(),
    }
}

async fn make(s: &Store, owner: &str) -> ScheduledView {
    s.create_scheduled(owner, new_at(START), T0).await.unwrap()
}

async fn outbox(pool: &SqlitePool) -> Vec<(String, Value)> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT user_id, payload FROM push_outbox ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    rows.into_iter()
        .map(|(u, p)| (u, serde_json::from_str(&p).unwrap()))
        .collect()
}

fn users_of(rows: &[(String, Value)], kind: &str) -> Vec<String> {
    let mut users: Vec<String> = rows
        .iter()
        .filter(|(_, p)| p["kind"] == kind)
        .map(|(u, _)| u.clone())
        .collect();
    users.sort();
    users
}

async fn clear_outbox(pool: &SqlitePool) {
    sqlx::query("DELETE FROM push_outbox")
        .execute(pool)
        .await
        .unwrap();
}

async fn view(s: &Store, token: &str, viewer: Option<&str>, now: i64) -> ScheduledView {
    s.scheduled_by_token(token, viewer, now)
        .await
        .unwrap()
        .unwrap()
}

fn names(v: &ScheduledView) -> Vec<Option<String>> {
    v.places.iter().map(|p| p.name.clone()).collect()
}

/// A match with four joined users: (store, pool, owner, view, [players]).
async fn full() -> (Store, SqlitePool, String, ScheduledView, Vec<String>) {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let v = make(&s, &owner).await;
    let mut who = Vec::new();
    for n in ["Ana", "Rui", "Marta", "Joana"] {
        let id = user(&s, n).await;
        s.join_scheduled(&v.join_token, &id, T0 + 1).await.unwrap();
        who.push(id);
    }
    (s, pool, owner, v, who)
}

#[tokio::test]
async fn play_later_creates_a_scheduled_match_not_a_live_one() {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let v = make(&s, &owner).await;
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(live, 0);
    assert!(!v.join_token.is_empty());
    assert_eq!(v.state, ScheduledState::Open);
    assert_eq!(v.owner_name, "Owner");
    assert_eq!(v.place.as_deref(), Some("Club court 2"));
    assert_eq!(v.places.len(), 4);
    assert!(v.places.iter().all(|p| p.name.is_none()));
    assert!(v.match_spectator_token.is_none());
    // Validation.
    assert!(matches!(
        s.create_scheduled(&owner, new_at(T0), T0).await,
        Err(StoreError::InvalidField("starts_at"))
    ));
    let mut long = new_at(START);
    long.place = Some("x".repeat(81));
    assert!(matches!(
        s.create_scheduled(&owner, long, T0).await,
        Err(StoreError::InvalidField("place"))
    ));
}

#[tokio::test]
async fn prefilled_places_from_the_owners_list() {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let other = user(&s, "Other").await;
    let (ana, _) = s.create_or_get_player(&owner, "Ana", 1).await.unwrap();
    let (rui, _) = s.create_or_get_player(&owner, "Rui", 1).await.unwrap();
    let (foreign, _) = s.create_or_get_player(&other, "Zed", 1).await.unwrap();
    let mut new = new_at(START);
    new.places = [Some(ana.id.clone()), None, Some(rui.id.clone()), None];
    let v = s.create_scheduled(&owner, new, T0).await.unwrap();
    assert_eq!(
        names(&v),
        vec![Some("Ana".to_owned()), None, Some("Rui".to_owned()), None]
    );
    assert_eq!(v.state, ScheduledState::Open);
    // Not the owner's, unknown, or the same twice.
    for places in [
        [Some(foreign.id), None, None, None],
        [Some("nope".to_owned()), None, None, None],
        [Some(ana.id.clone()), Some(ana.id.clone()), None, None],
    ] {
        let mut bad = new_at(START);
        bad.places = places;
        assert!(matches!(
            s.create_scheduled(&owner, bad, T0).await,
            Err(StoreError::InvalidPlayers(_))
        ));
    }
    // A linked entry copies its account into the place; only the owner sees the id.
    let u = user(&s, "Ula").await;
    let (ula, _) = s.create_or_get_player(&owner, "Ula", 1).await.unwrap();
    sqlx::query("UPDATE players SET linked_user_id = ? WHERE id = ?")
        .bind(&u)
        .bind(&ula.id)
        .execute(&pool)
        .await
        .unwrap();
    let mut linked = new_at(START);
    linked.places = [Some(ula.id), None, None, None];
    let v = s.create_scheduled(&owner, linked, T0).await.unwrap();
    assert_eq!(v.places[0].user_id.as_deref(), Some(u.as_str()));
    let anon = view(&s, &v.join_token, None, T0).await;
    assert_eq!(anon.places[0].user_id, None);
    assert_eq!(anon.places[0].name.as_deref(), Some("Ula"));
}

#[tokio::test]
async fn join_takes_the_next_open_place_then_the_line() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let (ana, _) = s.create_or_get_player(&owner, "Ana", 1).await.unwrap();
    let mut new = new_at(START);
    new.places = [None, Some(ana.id), None, None];
    let v = s.create_scheduled(&owner, new, T0).await.unwrap();
    let mut ids = Vec::new();
    for n in ["Rui", "Marta", "Joana", "Zoe", "Tom"] {
        ids.push(user(&s, n).await);
    }
    let t = &v.join_token;
    let after = s.join_scheduled(t, &ids[0], T0 + 1).await.unwrap();
    assert_eq!(after.me.as_ref().unwrap().slot, Some(0));
    s.join_scheduled(t, &ids[1], T0 + 2).await.unwrap();
    let third = s.join_scheduled(t, &ids[2], T0 + 3).await.unwrap();
    assert_eq!(third.me.as_ref().unwrap().slot, Some(3));
    assert_eq!(third.state, ScheduledState::Full);
    let fourth = s.join_scheduled(t, &ids[3], T0 + 4).await.unwrap();
    assert_eq!(fourth.me.as_ref().unwrap().slot, None);
    assert!(fourth.me.as_ref().unwrap().in_line);
    s.join_scheduled(t, &ids[4], T0 + 5).await.unwrap();
    let seen = view(&s, t, Some(&owner), T0 + 6).await;
    assert_eq!(seen.line.len(), 2);
    assert_eq!(seen.line[0].name.as_deref(), Some("Zoe"));
    assert_eq!(seen.line[0].slot, 0);
    assert_eq!(seen.line[1].slot, 1);
    assert!(seen.me.as_ref().unwrap().owner);
    // Joining twice is refused, in a place or the line.
    for u in [&ids[0], &ids[3]] {
        assert!(matches!(
            s.join_scheduled(t, u, T0 + 7).await,
            Err(StoreError::Conflict("already_in"))
        ));
    }
    // Strangers get no ids; anonymous viewers have no `me`.
    let anon = view(&s, t, None, T0 + 6).await;
    assert!(anon.me.is_none());
    assert!(anon.places.iter().all(|p| p.user_id.is_none()));
}

#[tokio::test]
async fn join_refused_when_already_placed_by_linked_entry() {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let u = user(&s, "Ula").await;
    let (ula, _) = s.create_or_get_player(&owner, "Ula", 1).await.unwrap();
    let (later, _) = s.create_or_get_player(&owner, "Later", 1).await.unwrap();
    sqlx::query("UPDATE players SET linked_user_id = ? WHERE id = ?")
        .bind(&u)
        .bind(&ula.id)
        .execute(&pool)
        .await
        .unwrap();
    let mut new = new_at(START);
    new.places = [Some(ula.id), Some(later.id.clone()), None, None];
    let v = s.create_scheduled(&owner, new, T0).await.unwrap();
    assert!(matches!(
        s.join_scheduled(&v.join_token, &u, T0 + 1).await,
        Err(StoreError::Conflict("already_in"))
    ));
    // The viewer sees their place through the entry.
    let mine = view(&s, &v.join_token, Some(&u), T0 + 1).await;
    assert_eq!(mine.me.unwrap().slot, Some(0));
    // An entry linked after the match was made counts too (P122).
    let w = user(&s, "Wes").await;
    sqlx::query("UPDATE players SET linked_user_id = ? WHERE id = ?")
        .bind(&w)
        .bind(&later.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        s.join_scheduled(&v.join_token, &w, T0 + 2).await,
        Err(StoreError::Conflict("already_in"))
    ));
}

#[tokio::test]
async fn leave_moves_the_first_in_line_up_outside_the_last_hour() {
    let (s, pool, _owner, v, who) = full().await;
    let t = &v.join_token;
    let q1 = user(&s, "Quin").await;
    let q2 = user(&s, "Quill").await;
    s.join_scheduled(t, &q1, T0 + 2).await.unwrap();
    s.join_scheduled(t, &q2, T0 + 3).await.unwrap();
    clear_outbox(&pool).await;
    s.leave_scheduled(t, &who[1], T0 + 10).await.unwrap();
    let seen = view(&s, t, None, T0 + 10).await;
    assert_eq!(seen.places[1].name.as_deref(), Some("Quin"));
    assert_eq!(seen.state, ScheduledState::Full);
    assert_eq!(seen.line.len(), 1);
    let rows = outbox(&pool).await;
    assert_eq!(users_of(&rows, "place_opened"), vec![q1.clone()]);
    let p = &rows[0].1;
    assert_eq!(p["name"], "Owner");
    assert_eq!(p["start"], START);
    assert!(p["url"].as_str().unwrap().starts_with("/j/"));
    // Leaving when not in is NotFound.
    assert!(matches!(
        s.leave_scheduled(t, &who[1], T0 + 11).await,
        Err(StoreError::NotFound)
    ));
    // Someone in the line can leave it.
    s.leave_scheduled(t, &q2, T0 + 12).await.unwrap();
    assert!(view(&s, t, None, T0 + 12).await.line.is_empty());
}

#[tokio::test]
async fn leave_inside_the_last_hour_keeps_the_place_open() {
    let (s, pool, _owner, v, who) = full().await;
    let t = &v.join_token;
    let q = user(&s, "Quin").await;
    s.join_scheduled(t, &q, T0 + 2).await.unwrap();
    clear_outbox(&pool).await;
    let now = START - HOUR + 1;
    s.leave_scheduled(t, &who[0], now).await.unwrap();
    let seen = view(&s, t, None, now).await;
    assert_eq!(seen.places[0].name, None);
    assert_eq!(seen.state, ScheduledState::Open);
    assert_eq!(seen.line.len(), 1);
    assert!(outbox(&pool).await.is_empty());
}

#[tokio::test]
async fn line_member_takes_open_place_inside_the_last_hour() {
    let (s, _pool, _owner, v, who) = full().await;
    let t = &v.join_token;
    let q = user(&s, "Quin").await;
    s.join_scheduled(t, &q, T0 + 2).await.unwrap();
    let now = START - HOUR + 1;
    s.leave_scheduled(t, &who[2], now).await.unwrap();
    let after = s.join_scheduled(t, &q, now + 1).await.unwrap();
    assert_eq!(after.me.as_ref().unwrap().slot, Some(2));
    assert!(!after.me.as_ref().unwrap().in_line);
    assert!(after.line.is_empty());
    // Placed users still get already_in.
    assert!(matches!(
        s.join_scheduled(t, &q, now + 2).await,
        Err(StoreError::Conflict("already_in"))
    ));
}

#[tokio::test]
async fn the_line_is_capped() {
    let (s, _pool, _owner, v, _who) = full().await;
    for i in 0..50 {
        let u = user(&s, &format!("Line{i}")).await;
        s.join_scheduled(&v.join_token, &u, T0 + 2).await.unwrap();
    }
    let one_more = user(&s, "Extra").await;
    assert!(matches!(
        s.join_scheduled(&v.join_token, &one_more, T0 + 3).await,
        Err(StoreError::Conflict("line_full"))
    ));
}

#[tokio::test]
async fn owner_edit_notifies_players_and_line() {
    let (s, pool, owner, v, who) = full().await;
    let t = &v.join_token;
    let q = user(&s, "Quin").await;
    s.join_scheduled(t, &q, T0 + 2).await.unwrap();
    clear_outbox(&pool).await;
    let out = s
        .update_scheduled(
            &owner,
            &v.id,
            ScheduledChange {
                starts_at: Some(START + HOUR),
                place: Some(None),
            },
            T0 + 10,
        )
        .await
        .unwrap();
    assert_eq!(out.starts_at, START + HOUR);
    assert_eq!(out.place, None);
    let rows = outbox(&pool).await;
    let mut want: Vec<String> = who.iter().cloned().chain([q.clone()]).collect();
    want.sort();
    assert_eq!(users_of(&rows, "match_changed"), want);
    assert_eq!(rows[0].1["start"], START + HOUR);
    // Past starts are refused; a stranger is NotFound.
    assert!(matches!(
        s.update_scheduled(
            &owner,
            &v.id,
            ScheduledChange {
                starts_at: Some(T0),
                place: None
            },
            T0 + 11
        )
        .await,
        Err(StoreError::InvalidField("starts_at"))
    ));
    assert!(matches!(
        s.update_scheduled(&who[0], &v.id, ScheduledChange::default(), T0 + 11)
            .await,
        Err(StoreError::NotFound)
    ));
    // Removing a player tells them and the line moves up.
    clear_outbox(&pool).await;
    s.remove_from_scheduled(&owner, &v.id, 0, T0 + 12)
        .await
        .unwrap();
    let rows = outbox(&pool).await;
    assert_eq!(users_of(&rows, "removed_from_match"), vec![who[0].clone()]);
    assert_eq!(users_of(&rows, "place_opened"), vec![q.clone()]);
    let seen = view(&s, t, None, T0 + 12).await;
    assert_eq!(seen.places[0].name.as_deref(), Some("Quin"));
    assert!(matches!(
        s.remove_from_scheduled(&owner, &v.id, 9, T0 + 13).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn cancelled_match_refuses_joins() {
    let (s, pool, owner, v, who) = full().await;
    let q = user(&s, "Quin").await;
    s.join_scheduled(&v.join_token, &q, T0 + 2).await.unwrap();
    clear_outbox(&pool).await;
    s.cancel_scheduled(&owner, &v.id, T0 + 5).await.unwrap();
    let rows = outbox(&pool).await;
    let mut want: Vec<String> = who.iter().cloned().chain([q]).collect();
    want.sort();
    assert_eq!(users_of(&rows, "match_cancelled"), want);
    let seen = view(&s, &v.join_token, None, T0 + 6).await;
    assert_eq!(seen.state, ScheduledState::Cancelled);
    let late = user(&s, "Late").await;
    assert!(matches!(
        s.join_scheduled(&v.join_token, &late, T0 + 6).await,
        Err(StoreError::Conflict("match_closed"))
    ));
    assert!(matches!(
        s.cancel_scheduled(&owner, &v.id, T0 + 7).await,
        Err(StoreError::Conflict("match_closed"))
    ));
    assert!(matches!(
        s.update_scheduled(&owner, &v.id, ScheduledChange::default(), T0 + 7)
            .await,
        Err(StoreError::Conflict("match_closed"))
    ));
}

#[tokio::test]
async fn start_needs_four_and_the_window() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let v = make(&s, &owner).await;
    let ids = [
        user(&s, "Ana").await,
        user(&s, "Rui").await,
        user(&s, "Marta").await,
    ];
    for u in &ids {
        s.join_scheduled(&v.join_token, u, T0 + 1).await.unwrap();
    }
    let ready = START - 1800;
    assert!(matches!(
        s.start_scheduled(&owner, &v.id, ready - 1).await,
        Err(StoreError::Conflict("not_yet"))
    ));
    assert!(matches!(
        s.start_scheduled(&owner, &v.id, START + 5400).await,
        Err(StoreError::Conflict("game_over"))
    ));
    assert!(matches!(
        s.start_scheduled(&owner, &v.id, ready).await,
        Err(StoreError::Conflict("match_short"))
    ));
    let fourth = user(&s, "Joana").await;
    s.join_scheduled(&v.join_token, &fourth, T0 + 2)
        .await
        .unwrap();
    assert!(matches!(
        s.start_scheduled(&ids[0], &v.id, ready).await,
        Err(StoreError::NotFound)
    ));
    let players = s.start_scheduled(&owner, &v.id, ready).await.unwrap();
    let got: Vec<&str> = players.players.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(got, vec!["Ana", "Rui", "Marta", "Joana"]);
    assert_eq!(players.format, Some(one_set()));
    assert!(matches!(players.from, StartedFrom::Scheduled { .. }));
}

fn new_match(owner: &str, players: &padel_score_server::starting::StartPlayers) -> NewMatch {
    let id = |i: usize| Some(players.players[i].player_id.clone());
    NewMatch {
        format: one_set(),
        start: Start {
            first_server: Player {
                team: Team::A,
                index: 0,
            },
            other_first_server: 0,
        },
        teams: Teams {
            a: ["x".into(), "y".into()],
            b: ["z".into(), "w".into()],
        },
        owner_user_id: Some(owner.to_owned()),
        players: padel_score_server::players::PlayerSlots {
            a: [id(0), id(1)],
            b: [id(2), id(3)],
        },
    }
}

#[tokio::test]
async fn match_from_scheduled_marks_it_started() {
    let (s, pool, owner, v, who) = full().await;
    let now = START;
    let players = s.start_scheduled(&owner, &v.id, now).await.unwrap();
    // Account places became entries of the owner's, linked automatically.
    let linked: Vec<Option<String>> = {
        let mut out = Vec::new();
        for p in &players.players {
            out.push(
                sqlx::query_scalar("SELECT linked_user_id FROM players WHERE id = ?")
                    .bind(&p.player_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
            );
        }
        out
    };
    let want: Vec<Option<String>> = who.iter().cloned().map(Some).collect();
    assert_eq!(linked, want);
    let from = players.from.clone();
    // Not the owner: NotFound.
    let stranger = user(&s, "Stranger").await;
    assert!(matches!(
        create_match_from(&s, new_match(&stranger, &players), &from, now).await,
        Err(StoreError::NotFound)
    ));
    // Outside the window.
    assert!(matches!(
        create_match_from(&s, new_match(&owner, &players), &from, START - 1801).await,
        Err(StoreError::Conflict("not_yet"))
    ));
    let m = create_match_from(&s, new_match(&owner, &players), &from, now)
        .await
        .unwrap();
    let seen = view(&s, &v.join_token, None, now + 1).await;
    assert_eq!(seen.state, ScheduledState::Started);
    assert_eq!(seen.match_spectator_token, Some(m.spectator_token));
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(live, 1);
    // A scheduled match starts at most one match.
    assert!(matches!(
        create_match_from(&s, new_match(&owner, &players), &from, now).await,
        Err(StoreError::Conflict("already_started"))
    ));
    assert!(matches!(
        s.cancel_scheduled(&owner, &v.id, now + 2).await,
        Err(StoreError::Conflict("match_closed"))
    ));
    let late = user(&s, "Late").await;
    assert!(matches!(
        s.join_scheduled(&v.join_token, &late, now + 2).await,
        Err(StoreError::Conflict("match_closed"))
    ));
    // Cancelled ones refuse with match_closed.
    let v2 = make(&s, &owner).await;
    s.cancel_scheduled(&owner, &v2.id, T0 + 1).await.unwrap();
    let fake = StartedFrom::Scheduled { id: v2.id.clone() };
    assert!(matches!(
        create_match_from(&s, new_match(&owner, &players), &fake, now).await,
        Err(StoreError::Conflict("match_closed"))
    ));
}

#[tokio::test]
async fn too_many_open_matches_are_refused() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    for _ in 0..50 {
        make(&s, &owner).await;
    }
    assert!(matches!(
        s.create_scheduled(&owner, new_at(START), T0).await,
        Err(StoreError::Conflict("too_many_scheduled"))
    ));
}

#[tokio::test]
async fn past_state_after_the_game_window() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let v = make(&s, &owner).await;
    let seen = view(&s, &v.join_token, None, START + 5400).await;
    assert_eq!(seen.state, ScheduledState::Past);
}

// ---- API ----

fn body_for(start: i64, listing: &str) -> Value {
    json!({ "starts_at": start, "place": "Club", "listing": listing, "format": one_set() })
}

#[tokio::test]
async fn public_and_private_both_open_by_link() {
    let (state, _pool) = state().await;
    let app = app_from(&state, 1);
    let (_o, owner) = fast_user(&state, "o@x.test", "Owner").await;
    let (_p, player) = fast_user(&state, "p@x.test", "Pat").await;
    let soon = now() + DAY;
    for listing in ["public", "private"] {
        let res = send(
            &app,
            as_session(
                &state,
                &owner,
                Method::POST,
                "/api/scheduled",
                Some(body_for(soon, listing)),
            ),
        )
        .await;
        assert_eq!(res.status, StatusCode::CREATED);
        assert_eq!(
            res.header(axum::http::header::CACHE_CONTROL),
            Some("no-store")
        );
        let token = res.body["join_token"].as_str().unwrap().to_owned();
        let anon = send(
            &app,
            req(Method::GET, &format!("/api/join/{token}"), None, &[]),
        )
        .await;
        assert_eq!(anon.status, StatusCode::OK);
        assert_eq!(anon.body["kind"], "match");
        assert_eq!(anon.body["view"]["state"], "open");
        assert!(anon.body["view"]["me"].is_null());
        // Partner sign-ups make no sense for a match.
        let partner = send(
            &app,
            as_session(
                &state,
                &player,
                Method::POST,
                &format!("/api/join/{token}"),
                Some(json!({"as": "partner"})),
            ),
        )
        .await;
        assert_eq!(partner.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(partner.body["field"], "as");
        let join = send(
            &app,
            as_session(
                &state,
                &player,
                Method::POST,
                &format!("/api/join/{token}"),
                Some(json!({"as": "solo"})),
            ),
        )
        .await;
        assert_eq!(join.status, StatusCode::OK);
        assert_eq!(join.body["me"]["slot"], 0);
        let again = send(
            &app,
            as_session(
                &state,
                &player,
                Method::POST,
                &format!("/api/join/{token}"),
                Some(json!({"as": "solo"})),
            ),
        )
        .await;
        assert_eq!(again.status, StatusCode::CONFLICT);
        assert_eq!(again.body["error"], "already_in");
        let leave = send(
            &app,
            as_session(
                &state,
                &player,
                Method::DELETE,
                &format!("/api/join/{token}"),
                None,
            ),
        )
        .await;
        assert_eq!(leave.status, StatusCode::NO_CONTENT);
    }
    let unknown = send(&app, req(Method::GET, "/api/join/nope", None, &[])).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn other_users_owner_routes_are_404() {
    let (state, _pool) = state().await;
    let app = app_from(&state, 2);
    let (_o, owner) = fast_user(&state, "o@x.test", "Owner").await;
    let (_p, other) = fast_user(&state, "p@x.test", "Pat").await;
    let created = send(
        &app,
        as_session(
            &state,
            &owner,
            Method::POST,
            "/api/scheduled",
            Some(body_for(now() + DAY, "private")),
        ),
    )
    .await;
    let id = created.body["id"].as_str().unwrap().to_owned();
    for (method, path, body) in [
        (
            Method::PATCH,
            format!("/api/scheduled/{id}"),
            json!({"place": null}),
        ),
        (
            Method::POST,
            format!("/api/scheduled/{id}/cancel"),
            json!({}),
        ),
        (
            Method::POST,
            format!("/api/scheduled/{id}/start"),
            json!({}),
        ),
        (
            Method::POST,
            format!("/api/scheduled/{id}/remove"),
            json!({"slot": 0}),
        ),
    ] {
        let res = send(&app, as_session(&state, &other, method, &path, Some(body))).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    let edit = send(
        &app,
        as_session(
            &state,
            &owner,
            Method::PATCH,
            &format!("/api/scheduled/{id}"),
            Some(json!({"place": null})),
        ),
    )
    .await;
    assert_eq!(edit.status, StatusCode::OK);
    assert!(edit.body["place"].is_null());
    let early = send(
        &app,
        as_session(
            &state,
            &owner,
            Method::POST,
            &format!("/api/scheduled/{id}/start"),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(early.status, StatusCode::CONFLICT);
    assert_eq!(early.body["error"], "not_yet");
    let cancel = send(
        &app,
        as_session(
            &state,
            &owner,
            Method::POST,
            &format!("/api/scheduled/{id}/cancel"),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(cancel.status, StatusCode::OK);
    assert_eq!(cancel.body["state"], "cancelled");
    let anon = send(
        &app,
        req(
            Method::POST,
            "/api/scheduled",
            Some(body_for(now() + DAY, "public")),
            &[],
        ),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
}

// ---- D5 fix wave ----

async fn link(pool: &SqlitePool, player: &str, user: &str) {
    sqlx::query("UPDATE players SET linked_user_id = ? WHERE id = ?")
        .bind(user)
        .bind(player)
        .execute(pool)
        .await
        .unwrap();
}

/// A full match whose slot 0 is a pre-filled entry "Pre" (not yet linked), slots 1-3 joined.
async fn with_prefilled() -> (
    Store,
    SqlitePool,
    String,
    ScheduledView,
    String,
    Vec<String>,
) {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let (pre, _) = s.create_or_get_player(&owner, "Pre", 1).await.unwrap();
    let mut new = new_at(START);
    new.places = [Some(pre.id.clone()), None, None, None];
    let v = s.create_scheduled(&owner, new, T0).await.unwrap();
    let mut who = Vec::new();
    for n in ["Ana", "Rui", "Marta"] {
        let id = user(&s, n).await;
        s.join_scheduled(&v.join_token, &id, T0 + 1).await.unwrap();
        who.push(id);
    }
    (s, pool, owner, v, pre.id, who)
}

#[tokio::test]
async fn line_member_linked_later_is_not_promoted_twice() {
    let (s, pool, _owner, v, pre, who) = with_prefilled().await;
    let t = &v.join_token;
    let dup = user(&s, "Dup").await;
    let next = user(&s, "Next").await;
    s.join_scheduled(t, &dup, T0 + 2).await.unwrap();
    s.join_scheduled(t, &next, T0 + 3).await.unwrap();
    // Dup is first in line; the owner's pre-filled entry then gets linked to them.
    link(&pool, &pre, &dup).await;
    clear_outbox(&pool).await;
    s.leave_scheduled(t, &who[0], T0 + 10).await.unwrap();
    let seen = view(&s, t, None, T0 + 10).await;
    assert_eq!(seen.places[1].name.as_deref(), Some("Next"));
    assert!(seen.line.is_empty());
    assert_eq!(users_of(&outbox(&pool).await, "place_opened"), vec![next]);
}

#[tokio::test]
async fn start_refuses_the_same_person_twice() {
    let (s, pool, owner, v, pre, _who) = with_prefilled().await;
    let ana: String = sqlx::query_scalar("SELECT id FROM users WHERE display_name = 'Ana'")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Linked after Ana joined: the pre-filled entry is Ana again.
    link(&pool, &pre, &ana).await;
    assert!(matches!(
        s.start_scheduled(&owner, &v.id, START).await,
        Err(StoreError::Conflict("same_player_twice"))
    ));
}

#[tokio::test]
async fn edit_later_fills_open_places_from_the_line() {
    let (s, _pool, owner, v, who) = full().await;
    let t = &v.join_token;
    let q = user(&s, "Quin").await;
    s.join_scheduled(t, &q, T0 + 2).await.unwrap();
    // Inside the last hour a place opens and stays open.
    let now = START - HOUR + 1;
    s.leave_scheduled(t, &who[0], now).await.unwrap();
    assert_eq!(view(&s, t, None, now).await.places[0].name, None);
    // Moving the match two hours later reopens the line's claim.
    let out = s
        .update_scheduled(
            &owner,
            &v.id,
            ScheduledChange {
                starts_at: Some(START + 2 * HOUR),
                place: None,
            },
            now,
        )
        .await
        .unwrap();
    assert_eq!(out.places[0].name.as_deref(), Some("Quin"));
    assert!(out.line.is_empty());
}

#[tokio::test]
async fn the_line_is_first_come_within_one_second() {
    let (s, _pool, _owner, v, who) = full().await;
    let t = &v.join_token;
    // Join in descending id order, so an id tie-break would always put them in reverse.
    let mut users = Vec::new();
    for i in 0..6 {
        users.push((user(&s, &format!("Same{i}")).await, format!("Same{i}")));
    }
    users.sort_by(|a, b| b.0.cmp(&a.0));
    for (id, _) in &users {
        s.join_scheduled(t, id, T0 + 2).await.unwrap();
    }
    for i in 0..3 {
        s.leave_scheduled(t, &who[i], T0 + 10).await.unwrap();
        let seen = view(&s, t, None, T0 + 10).await;
        assert_eq!(seen.places[i].name.as_deref(), Some(users[i].1.as_str()));
    }
}

#[tokio::test]
async fn starting_a_short_match_over_http_is_refused() {
    let (state, _pool) = state().await;
    let app = app_from(&state, 3);
    let (owner_id, owner) = fast_user(&state, "o@x.test", "Owner").await;
    let t = now();
    let created = state
        .store
        .create_scheduled(&owner_id, new_at(t + 600), t)
        .await
        .unwrap();
    let body = json!({
        "format": one_set(),
        "start": Start { first_server: Player { team: Team::A, index: 0 }, other_first_server: 0 },
        "teams": { "a": ["A", "B"], "b": ["C", "D"] },
        "from": { "kind": "scheduled", "id": created.id },
    });
    let res = send(
        &app,
        as_session(&state, &owner, Method::POST, "/api/matches", Some(body)),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "match_short");
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&_pool)
        .await
        .unwrap();
    assert_eq!(live, 0);
}

// ---- deferred minors: B2 and B3 ----

/// Slot 0 is an entry "Ula" of the owner's linked to a new account; slots 1-2 are joined users,
/// so three places are filled. Returns (store, pool, owner, view, ula entry, ula account).
async fn with_linked_entry() -> (Store, SqlitePool, String, ScheduledView, String, String) {
    let (s, pool) = setup().await;
    let owner = user(&s, "Owner").await;
    let ula = user(&s, "Ula").await;
    let (entry, _) = s.create_or_get_player(&owner, "Ula", 1).await.unwrap();
    link(&pool, &entry.id, &ula).await;
    let mut new = new_at(START);
    new.places = [Some(entry.id.clone()), None, None, None];
    let v = s.create_scheduled(&owner, new, T0).await.unwrap();
    for n in ["Ana", "Rui"] {
        let id = user(&s, n).await;
        s.join_scheduled(&v.join_token, &id, T0 + 1).await.unwrap();
    }
    (s, pool, owner, v, entry.id, ula)
}

async fn calendar_match_players(s: &Store, user: &str) -> Option<Vec<Option<String>>> {
    let cal = s.calendar(user, 0, T0).await.unwrap();
    cal.items.into_iter().find_map(|i| match i {
        padel_score_server::calendar::CalendarItem::Match { players, .. } => Some(players),
        _ => None,
    })
}

#[tokio::test]
async fn deleting_the_started_match_makes_the_scheduled_one_startable_again() {
    let (s, pool, owner, v, _who) = full().await;
    let players = s.start_scheduled(&owner, &v.id, START).await.unwrap();
    let from = players.from.clone();
    let m = create_match_from(&s, new_match(&owner, &players), &from, START)
        .await
        .unwrap();
    assert_eq!(
        view(&s, &v.join_token, None, START + 1).await.state,
        ScheduledState::Started
    );
    sqlx::query("DELETE FROM matches WHERE id = ?")
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();
    // Not started any more: back in the window, joinable state, startable again.
    let seen = view(&s, &v.join_token, None, START + 2).await;
    assert_eq!(seen.state, ScheduledState::Full);
    assert_eq!(seen.match_spectator_token, None);
    let again = s.start_scheduled(&owner, &v.id, START + 3).await.unwrap();
    let m2 = create_match_from(&s, new_match(&owner, &again), &from, START + 3)
        .await
        .unwrap();
    assert_ne!(m2.id, m.id);
    assert_eq!(
        view(&s, &v.join_token, None, START + 4).await.state,
        ScheduledState::Started
    );
}

#[tokio::test]
async fn deleting_the_started_match_brings_back_the_short_match_reminder() {
    let (s, pool, owner, v, _who) = full().await;
    let players = s.start_scheduled(&owner, &v.id, START).await.unwrap();
    let from = players.from.clone();
    let m = create_match_from(&s, new_match(&owner, &players), &from, START)
        .await
        .unwrap();
    // Three filled places starting within a day are reminded of, unless the match is started.
    sqlx::query("DELETE FROM scheduled_places WHERE scheduled_id = ? AND slot = 0")
        .bind(&v.id)
        .execute(&pool)
        .await
        .unwrap();
    let before = START - HOUR;
    clear_outbox(&pool).await;
    assert_eq!(s.queue_reminders(before).await.unwrap(), 0);
    sqlx::query("DELETE FROM matches WHERE id = ?")
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(s.queue_reminders(before).await.unwrap() > 0);
    assert!(!users_of(&outbox(&pool).await, "match_short").is_empty());
}

#[tokio::test]
async fn unlinking_an_entry_takes_the_account_out_of_the_place() {
    let (s, pool, owner, v, entry, ula) = with_linked_entry().await;
    // Linked: the account sees the match on its calendar and is told about changes.
    assert!(calendar_match_players(&s, &ula).await.is_some());
    sqlx::query("UPDATE players SET linked_user_id = NULL WHERE id = ?")
        .bind(&entry)
        .execute(&pool)
        .await
        .unwrap();
    let seen = view(&s, &v.join_token, Some(&owner), T0).await;
    assert_eq!(seen.places[0].user_id, None);
    assert_eq!(seen.places[0].name.as_deref(), Some("Ula"));
    assert_eq!(
        view(&s, &v.join_token, Some(&ula), T0)
            .await
            .me
            .unwrap()
            .slot,
        None
    );
    assert!(calendar_match_players(&s, &ula).await.is_none());
    clear_outbox(&pool).await;
    s.update_scheduled(
        &owner,
        &v.id,
        ScheduledChange {
            starts_at: Some(START + HOUR),
            place: None,
        },
        T0 + 10,
    )
    .await
    .unwrap();
    assert!(!users_of(&outbox(&pool).await, "match_changed").contains(&ula));
    // The account is free to take a place itself again.
    s.join_scheduled(&v.join_token, &ula, T0 + 11)
        .await
        .unwrap();
}

#[tokio::test]
async fn relinking_an_entry_shows_the_new_account_as_a_player() {
    let (s, pool, owner, v, entry, ula) = with_linked_entry().await;
    sqlx::query("UPDATE players SET linked_user_id = NULL WHERE id = ?")
        .bind(&entry)
        .execute(&pool)
        .await
        .unwrap();
    let other = user(&s, "Other").await;
    link(&pool, &entry, &other).await;
    let seen = view(&s, &v.join_token, Some(&owner), T0).await;
    assert_eq!(seen.places[0].user_id.as_deref(), Some(other.as_str()));
    assert!(calendar_match_players(&s, &other).await.is_some());
    assert!(calendar_match_players(&s, &ula).await.is_none());
    clear_outbox(&pool).await;
    s.cancel_scheduled(&owner, &v.id, T0 + 10).await.unwrap();
    let told = users_of(&outbox(&pool).await, "match_cancelled");
    assert!(told.contains(&other));
    assert!(!told.contains(&ula));
}
