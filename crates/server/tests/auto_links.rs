#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::http::{Method, StatusCode};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::db;
use padel_score_server::linking::auto_entry_in;
use padel_score_server::players::PlayerSlots;
use padel_score_server::privacy::{Viewer, Visibility, viewer_of};
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{as_session, fragment_code, invited_user, send, with_admin};

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

async fn user(s: &Store, email: &str, name: &str) -> String {
    s.create_user(
        NewUser {
            email: email.to_owned(),
            display_name: name.to_owned(),
            password_hash: "x".to_owned(),
        },
        1,
    )
    .await
    .unwrap()
    .id
}

async fn auto(s: &Store, owner: &str, user_id: &str) -> Result<String, StoreError> {
    let mut tx = s.begin().await?;
    let id = auto_entry_in(&mut tx, owner, user_id, 5).await?;
    tx.commit().await?;
    Ok(id)
}

async fn flag(pool: &SqlitePool, id: &str) -> (i64, Option<String>, Option<i64>) {
    sqlx::query_as("SELECT link_auto, linked_user_id, archived_at FROM players WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn name_of(pool: &SqlitePool, id: &str) -> String {
    sqlx::query_scalar("SELECT name FROM players WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn auto_entry_creates_a_linked_entry_named_after_the_user() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = user(&s, "rui@example.com", "Rui Costa").await;
    let id = auto(&s, &ana, &rui).await.unwrap();
    assert_eq!(name_of(&pool, &id).await, "Rui Costa");
    let (auto_mark, linked, _) = flag(&pool, &id).await;
    assert_eq!((auto_mark, linked), (1, Some(rui.clone())));
    let links = s.links(&ana).await.unwrap();
    assert!(links[&id].automatic);
    assert_eq!(links[&id].linked_at, 5);
}

#[tokio::test]
async fn auto_entry_for_the_owner_themself_is_not_automatic() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let id = auto(&s, &ana, &ana).await.unwrap();
    assert_eq!(flag(&pool, &id).await.0, 0);
}

#[tokio::test]
async fn auto_entry_reuses_an_existing_linked_entry() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = user(&s, "rui@example.com", "Rui").await;
    let manual = s
        .create_or_get_player(&ana, "Ruizinho", 1)
        .await
        .unwrap()
        .0
        .id;
    sqlx::query("UPDATE players SET linked_user_id = ?, linked_at = 2 WHERE id = ?")
        .bind(&rui)
        .bind(&manual)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(auto(&s, &ana, &rui).await.unwrap(), manual);
    assert_eq!(flag(&pool, &manual).await.0, 0);
    // A second call reuses an automatic entry too.
    let sam = user(&s, "sam@example.com", "Sam").await;
    let first = auto(&s, &ana, &sam).await.unwrap();
    assert_eq!(auto(&s, &ana, &sam).await.unwrap(), first);
    assert_eq!(flag(&pool, &first).await.0, 1);
}

#[tokio::test]
async fn auto_entry_prefers_unarchived_then_unarchives_the_newest_archived() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = user(&s, "rui@example.com", "Rui").await;
    let old = auto(&s, &ana, &rui).await.unwrap();
    let newer = s.create_or_get_player(&ana, "Rui B", 9).await.unwrap().0.id;
    sqlx::query("UPDATE players SET linked_user_id = ?, linked_at = 9 WHERE id = ?")
        .bind(&rui)
        .bind(&newer)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE players SET archived_at = 20 WHERE id IN (?, ?)")
        .bind(&old)
        .bind(&newer)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(auto(&s, &ana, &rui).await.unwrap(), newer);
    assert_eq!(flag(&pool, &newer).await.2, None);
    assert!(flag(&pool, &old).await.2.is_some());
    // Now `newer` is unarchived: it wins over the archived one even after another call.
    assert_eq!(auto(&s, &ana, &rui).await.unwrap(), newer);
}

#[tokio::test]
async fn auto_entry_adds_a_number_on_a_name_clash() {
    let (s, pool) = setup().await;
    let owner = user(&s, "owner@example.com", "Owner").await;
    let a1 = user(&s, "ana1@example.com", "Ana").await;
    let a2 = user(&s, "ana2@example.com", "ana").await;
    s.create_or_get_player(&owner, "Ana", 1).await.unwrap();
    let first = auto(&s, &owner, &a1).await.unwrap();
    assert_eq!(name_of(&pool, &first).await, "Ana 2");
    let second = auto(&s, &owner, &a2).await.unwrap();
    assert_eq!(name_of(&pool, &second).await, "ana 3");
}

#[tokio::test]
async fn auto_entry_truncates_a_long_name_to_fit_the_number() {
    let (s, pool) = setup().await;
    let owner = user(&s, "owner@example.com", "Owner").await;
    let long = "x".repeat(40);
    let u = user(&s, "long@example.com", &long).await;
    s.create_or_get_player(&owner, &long, 1).await.unwrap();
    let id = auto(&s, &owner, &u).await.unwrap();
    let name = name_of(&pool, &id).await;
    assert_eq!(name, format!("{} 2", "x".repeat(38)));
    assert_eq!(name.chars().count(), 40);
}

#[tokio::test]
async fn auto_entry_gives_name_taken_after_ninety_nine() {
    let (s, _pool) = setup().await;
    let owner = user(&s, "owner@example.com", "Owner").await;
    let u = user(&s, "ana@example.com", "Ana").await;
    s.create_or_get_player(&owner, "Ana", 1).await.unwrap();
    for n in 2..=99 {
        s.create_or_get_player(&owner, &format!("Ana {n}"), 1)
            .await
            .unwrap();
    }
    assert!(matches!(
        auto(&s, &owner, &u).await,
        Err(StoreError::Conflict("name_taken"))
    ));
}

struct World {
    state: padel_score_server::AppState,
    pool: SqlitePool,
    app: axum::Router,
    ana_id: String,
    ana: String,
    rui_id: String,
    rui: String,
    entry: String,
}

async fn world() -> World {
    let (state, pool, app, ana_id, ana) = with_admin().await;
    let (rui_id, rui) = invited_user(&state, &app, &ana, "rui@example.com", "Rui").await;
    let mut tx = state.store.begin().await.unwrap();
    let entry = auto_entry_in(&mut tx, &ana_id, &rui_id, 5).await.unwrap();
    tx.commit().await.unwrap();
    World {
        state,
        pool,
        app,
        ana_id,
        ana,
        rui_id,
        rui,
        entry,
    }
}

impl World {
    async fn call(
        &self,
        who: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> support::Res {
        send(&self.app, as_session(&self.state, who, method, path, body)).await
    }
}

#[tokio::test]
async fn auto_link_does_not_open_combined_history() {
    let w = world().await;
    w.state
        .store
        .set_visibility(&w.rui_id, Visibility::Coordinators)
        .await
        .unwrap();
    let viewer = viewer_of(&w.state.store, &w.rui_id, Some(&w.ana_id), false)
        .await
        .unwrap();
    assert_eq!(viewer, None);
    let res = w
        .call(
            &w.ana,
            Method::GET,
            &format!("/api/players/{}/linked-history", w.entry),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN, "{}", res.body);
    assert_eq!(res.body["error"], "history_private");
}

#[tokio::test]
async fn link_code_on_an_auto_entry_clears_the_mark() {
    let w = world().await;
    w.state
        .store
        .set_visibility(&w.rui_id, Visibility::Coordinators)
        .await
        .unwrap();
    let made = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-code", w.entry),
            None,
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let code = fragment_code(made.body["url"].as_str().unwrap());
    let looked = w
        .call(
            &w.rui,
            Method::POST,
            "/api/link-codes/lookup",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(looked.status, StatusCode::OK, "{}", looked.body);
    let accepted = w
        .call(
            &w.rui,
            Method::POST,
            "/api/link-codes/accept",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    let (mark, linked, _) = flag(&w.pool, &w.entry).await;
    assert_eq!((mark, linked), (0, Some(w.rui_id.clone())));
    let viewer = viewer_of(&w.state.store, &w.rui_id, Some(&w.ana_id), false)
        .await
        .unwrap();
    assert_eq!(
        viewer,
        Some(Viewer::Coordinator {
            id: w.ana_id.clone()
        })
    );
}

#[tokio::test]
async fn link_code_on_an_entry_linked_to_someone_else_is_refused() {
    let w = world().await;
    let (_, other) = invited_user(&w.state, &w.app, &w.ana, "sam@example.com", "Sam").await;
    let made = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-code", w.entry),
            None,
        )
        .await;
    let code = fragment_code(made.body["url"].as_str().unwrap());
    let res = w
        .call(
            &other,
            Method::POST,
            "/api/link-codes/accept",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(flag(&w.pool, &w.entry).await.0, 1);
}

fn format() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

#[tokio::test]
async fn claim_on_an_auto_entry_needs_approval_then_clears_the_mark() {
    let w = world().await;
    let m = w
        .state
        .store
        .create_match(
            NewMatch {
                format: format(),
                start: Start {
                    first_server: Player {
                        team: Team::A,
                        index: 0,
                    },
                    other_first_server: 0,
                },
                teams: Teams {
                    a: ["A1".into(), "A2".into()],
                    b: ["B1".into(), "B2".into()],
                },
                owner_user_id: Some(w.ana_id.clone()),
                players: PlayerSlots {
                    a: [Some(w.entry.clone()), None],
                    b: [None, None],
                },
            },
            1,
        )
        .await
        .unwrap();
    let target = json!({ "kind": "match_slot", "spectator_token": m.spectator_token,
                         "team": "a", "position": 0 });
    let claimed = w
        .call(&w.rui, Method::POST, "/api/claims", Some(target))
        .await;
    assert_eq!(claimed.status, StatusCode::CREATED, "{}", claimed.body);
    assert_eq!(claimed.body["status"], "pending");
    // Still automatic, and the coordinator sees the claim.
    assert_eq!(flag(&w.pool, &w.entry).await.0, 1);
    let inbox = w.call(&w.ana, Method::GET, "/api/claims", None).await;
    let items = inbox.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{}", inbox.body);
    let listed = w.call(&w.ana, Method::GET, "/api/players", None).await;
    let row = listed.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == w.entry.as_str())
        .unwrap();
    assert_eq!(row["pending_claims"], 1);
    let id = items[0]["id"].as_str().unwrap();
    let approved = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/claims/{id}/approve"),
            None,
        )
        .await;
    assert!(approved.status.is_success(), "{}", approved.body);
    let (mark, linked, _) = flag(&w.pool, &w.entry).await;
    assert_eq!((mark, linked), (0, Some(w.rui_id.clone())));
    // Now it is theirs.
    let again = w
        .call(
            &w.rui,
            Method::POST,
            "/api/claims",
            Some(json!({ "kind": "match_slot",
            "spectator_token": m.spectator_token, "team": "a", "position": 0 })),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.body);
    assert_eq!(again.body["error"], "already_yours");
}

#[tokio::test]
async fn players_list_shows_automatic_link() {
    let w = world().await;
    let manual = w
        .state
        .store
        .create_or_get_player(&w.ana_id, "Bea", 1)
        .await
        .unwrap()
        .0
        .id;
    w.state.store.link_me(&w.ana_id, &manual, 2).await.unwrap();
    let res = w.call(&w.ana, Method::GET, "/api/players", None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let items = res.body["items"].as_array().unwrap();
    let link_of = |id: &str| items.iter().find(|p| p["id"] == id).unwrap()["link"].clone();
    assert_eq!(link_of(&w.entry)["automatic"], true);
    assert_eq!(link_of(&manual)["automatic"], false);
}
