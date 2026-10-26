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
use padel_score_server::competitions::{Kind, NewCompetition};
use padel_score_server::players::PlayerSlots;
use padel_score_server::store::{NewMatch, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Res, as_device, as_session, device_token, invited_user, req, send, with_admin};

/// Ana (admin, coordinator) with entries Rui and Sam, and two other accounts R1 and R2.
struct World {
    state: AppState,
    pool: SqlitePool,
    app: Router,
    ana_id: String,
    ana: String,
    r1_id: String,
    r1: String,
    r2_id: String,
    r2: String,
    rui: String,
    sam: String,
    /// Ana's owned match: Rui and Sam on team a, free text on team b.
    token: String,
}

fn format() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn make_match(
    state: &AppState,
    owner: Option<&str>,
    a: [Option<&str>; 2],
    b: [Option<&str>; 2],
) -> (String, String) {
    let slot = |ids: [Option<&str>; 2]| ids.map(|id| id.map(str::to_owned));
    let m = state
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
                owner_user_id: owner.map(str::to_owned),
                players: PlayerSlots {
                    a: slot(a),
                    b: slot(b),
                },
            },
            1,
        )
        .await
        .unwrap();
    (m.id, m.spectator_token)
}

async fn world() -> World {
    let (state, pool, app, ana_id, ana) = with_admin().await;
    let (r1_id, r1) = invited_user(&state, &app, &ana, "r1@example.com", "Rui Costa").await;
    let (r2_id, r2) = invited_user(&state, &app, &ana, "r2@example.com", "Rui Silva").await;
    let rui = state
        .store
        .create_or_get_player(&ana_id, "Rui", 1)
        .await
        .unwrap()
        .0
        .id;
    let sam = state
        .store
        .create_or_get_player(&ana_id, "Sam", 1)
        .await
        .unwrap()
        .0
        .id;
    let (_, token) = make_match(
        &state,
        Some(&ana_id),
        [Some(&rui), Some(&sam)],
        [None, None],
    )
    .await;
    World {
        state,
        pool,
        app,
        ana_id,
        ana,
        r1_id,
        r1,
        r2_id,
        r2,
        rui,
        sam,
        token,
    }
}

impl World {
    async fn call(&self, who: &str, method: Method, path: &str, body: Option<Value>) -> Res {
        send(&self.app, as_session(&self.state, who, method, path, body)).await
    }

    fn target(&self, position: u8) -> Value {
        json!({ "kind": "match_slot", "spectator_token": self.token, "team": "a",
                "position": position })
    }

    async fn claim(&self, who: &str, position: u8) -> Res {
        self.call(
            who,
            Method::POST,
            "/api/claims",
            Some(self.target(position)),
        )
        .await
    }

    async fn inbox(&self) -> Vec<Value> {
        let res = self.call(&self.ana, Method::GET, "/api/claims", None).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body["items"].as_array().unwrap().clone()
    }

    async fn approve(&self, who: &str, id: &str) -> Res {
        let path = format!("/api/claims/{id}/approve");
        self.call(who, Method::POST, &path, None).await
    }

    async fn decline(&self, who: &str, id: &str) -> Res {
        let path = format!("/api/claims/{id}/decline");
        self.call(who, Method::POST, &path, None).await
    }

    async fn player_list(&self) -> Vec<Value> {
        let res = self
            .call(&self.ana, Method::GET, "/api/players", None)
            .await;
        res.body["items"].as_array().unwrap().clone()
    }
}

fn keys(v: &Value) -> Vec<&str> {
    let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    k.sort_unstable();
    k
}

#[tokio::test]
async fn claim_round_trip() {
    let w = world().await;
    let made = w.claim(&w.r1, 0).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    assert_eq!(made.body["status"], "pending");

    let inbox = w.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["player"], json!({ "id": w.rui, "name": "Rui" }));
    assert_eq!(
        inbox[0]["claimant"],
        json!({ "display_name": "Rui Costa", "email": "r1@example.com" })
    );
    assert_eq!(inbox[0]["preview"]["name"], "Rui");
    let players = w.player_list().await;
    let rui = players.iter().find(|p| p["id"] == w.rui.as_str()).unwrap();
    assert_eq!(rui["pending_claims"], 1);
    assert_eq!(rui["link"], Value::Null);

    let id = inbox[0]["id"].as_str().unwrap();
    let res = w.approve(&w.ana, id).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(w.inbox().await.is_empty());

    let links = w.call(&w.r1, Method::GET, "/api/me/links", None).await;
    assert_eq!(links.status, StatusCode::OK);
    let items = links.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["player_id"], w.rui.as_str());
    assert_eq!(items[0]["name"], "Rui");
    assert_eq!(items[0]["coordinator_name"], "Ana");

    let players = w.player_list().await;
    let rui = players.iter().find(|p| p["id"] == w.rui.as_str()).unwrap();
    assert_eq!(rui["link"]["display_name"], "Rui Costa");
    assert_eq!(rui["link"]["is_me"], false);
    assert_eq!(rui["pending_claims"], 0);
    let mine = w.call(&w.r1, Method::GET, "/api/me/claims", None).await;
    assert_eq!(mine.body["items"][0]["status"], "approved");

    // The claimant can unlink themselves.
    let path = format!("/api/me/links/{}", w.rui);
    let res = w.call(&w.r1, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = w.call(&w.r1, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn preview_shows_only_name_counts_dates() {
    let w = world().await;
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims/preview",
            Some(w.target(0)),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        keys(&res.body),
        [
            "coordinator_name",
            "events",
            "first_at",
            "last_at",
            "matches",
            "name"
        ]
    );
    assert_eq!(res.body["name"], "Rui");
    assert_eq!(res.body["coordinator_name"], "Ana");
}

#[tokio::test]
async fn my_pending_claims_show_no_results() {
    let w = world().await;
    w.claim(&w.r1, 0).await;
    let res = w.call(&w.r1, Method::GET, "/api/me/claims", None).await;
    assert_eq!(res.status, StatusCode::OK);
    let items = res.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        keys(&items[0]),
        ["created_at", "decided_at", "id", "preview", "status"]
    );
    assert_eq!(
        keys(&items[0]["preview"]),
        [
            "coordinator_name",
            "events",
            "first_at",
            "last_at",
            "matches",
            "name"
        ]
    );
    let text = res.body.to_string();
    assert!(!text.contains(&w.token), "{text}");
    assert!(!text.contains("Sam"), "{text}");
}

#[tokio::test]
async fn slots_show_claim_states() {
    let w = world().await;
    // Team a: Rui (position 0) and Sam (position 1); team b: free text, one picked player.
    let bia = w
        .state
        .store
        .create_or_get_player(&w.ana_id, "Bia", 1)
        .await
        .unwrap()
        .0
        .id;
    let cat = w
        .state
        .store
        .create_or_get_player(&w.ana_id, "Cat", 1)
        .await
        .unwrap()
        .0
        .id;
    let dan = w
        .state
        .store
        .create_or_get_player(&w.ana_id, "Dan", 1)
        .await
        .unwrap()
        .0
        .id;
    let (_, token) = make_match(
        &w.state,
        Some(&w.ana_id),
        [Some(&w.rui), Some(&w.sam)],
        [Some(&bia), None],
    )
    .await;
    // Dan and Cat in a second match are irrelevant; use R1's perspective on states:
    // Rui: R1's pending claim; Sam: linked to R2; Bia: declined for R1; free text unlisted.
    let (_, other) = make_match(
        &w.state,
        Some(&w.ana_id),
        [Some(&cat), Some(&dan)],
        [None, None],
    )
    .await;
    let claim = |tok: &str, team: &str, pos: u8| json!({ "kind": "match_slot", "spectator_token": tok, "team": team, "position": pos });
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims",
            Some(claim(&token, "a", 0)),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims",
            Some(claim(&token, "b", 0)),
        )
        .await;
    let bia_claim = res.body["id"].as_str().unwrap().to_owned();
    w.decline(&w.ana, &bia_claim).await;
    // Sam linked to R2 through a claim and approval.
    let res = w
        .call(
            &w.r2,
            Method::POST,
            "/api/claims",
            Some(claim(&token, "a", 1)),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED);
    w.approve(&w.ana, res.body["id"].as_str().unwrap()).await;

    let res = w
        .call(
            &w.r1,
            Method::GET,
            &format!("/api/view/{token}/slots"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body["items"],
        json!([
            { "team": "a", "position": 0, "name": "Rui", "state": "pending" },
            { "team": "a", "position": 1, "name": "Sam", "state": "linked" },
            { "team": "b", "position": 0, "name": "Bia", "state": "declined" },
        ])
    );
    // R2's view: Sam is theirs, Rui claimable; and the other match is all claimable.
    let res = w
        .call(
            &w.r2,
            Method::GET,
            &format!("/api/view/{token}/slots"),
            None,
        )
        .await;
    assert_eq!(res.body["items"][0]["state"], "claimable");
    assert_eq!(res.body["items"][1]["state"], "yours");
    let res = w
        .call(
            &w.r1,
            Method::GET,
            &format!("/api/view/{other}/slots"),
            None,
        )
        .await;
    let states: Vec<_> = res.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["state"].clone())
        .collect();
    assert_eq!(states, [json!("claimable"), json!("claimable")]);
}

#[tokio::test]
async fn anonymous_match_has_no_claimable_slots() {
    let w = world().await;
    let (_, token) = make_match(&w.state, None, [None, None], [None, None]).await;
    let target = json!({ "kind": "match_slot", "spectator_token": token, "team": "a",
        "position": 0 });
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims/preview",
            Some(target.clone()),
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"], "not_found");
    let res = w
        .call(&w.r1, Method::POST, "/api/claims", Some(target))
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = w
        .call(
            &w.r1,
            Method::GET,
            &format!("/api/view/{token}/slots"),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body, json!({ "items": [] }));
}

#[tokio::test]
async fn free_text_slot_is_404() {
    let w = world().await;
    let target = json!({ "kind": "match_slot", "spectator_token": w.token, "team": "b",
        "position": 0 });
    for path in ["/api/claims/preview", "/api/claims"] {
        let res = w
            .call(&w.r1, Method::POST, path, Some(target.clone()))
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    // A slot position that does not exist is the same answer.
    let res = w.claim(&w.r1, 7).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_spectator_token_is_404() {
    let w = world().await;
    let target = json!({ "kind": "match_slot", "spectator_token": "nope", "team": "a",
        "position": 0 });
    for path in ["/api/claims/preview", "/api/claims"] {
        let res = w
            .call(&w.r1, Method::POST, path, Some(target.clone()))
            .await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    let res = w
        .call(&w.r1, Method::GET, "/api/view/nope/slots", None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_claim_links_at_once() {
    let w = world().await;
    let res = w.claim(&w.ana, 0).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["status"], "approved");
    assert!(res.body["decided_at"].as_i64().is_some());
    let links = w.state.store.links(&w.ana_id).await.unwrap();
    assert_eq!(links[&w.rui].user_id, w.ana_id);
    let players = w.player_list().await;
    let rui = players.iter().find(|p| p["id"] == w.rui.as_str()).unwrap();
    assert_eq!(rui["link"]["is_me"], true);
    // Claiming it again says it is already theirs.
    let res = w.claim(&w.ana, 0).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "already_yours");
    // Others see it as linked.
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "player_linked");
}

#[tokio::test]
async fn decline_keeps_plain_name_and_blocks_reclaim() {
    let w = world().await;
    let made = w.claim(&w.r1, 0).await;
    let id = made.body["id"].as_str().unwrap();
    let res = w.decline(&w.ana, id).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(w.state.store.links(&w.ana_id).await.unwrap().is_empty());
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "claim_declined");
    // Deciding twice is a conflict.
    let res = w.decline(&w.ana, id).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "claim_decided");
    let res = w.approve(&w.ana, id).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn withdraw_then_reclaim_is_allowed() {
    let w = world().await;
    let made = w.claim(&w.r1, 0).await;
    assert_eq!(made.status, StatusCode::CREATED);
    let id = made.body["id"].as_str().unwrap().to_owned();
    // An existing pending claim comes back as it is.
    let again = w.claim(&w.r1, 0).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body["id"], id.as_str());
    let path = format!("/api/me/claims/{id}");
    // Another account cannot withdraw it.
    let res = w.call(&w.r2, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = w.call(&w.r1, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(w.inbox().await.is_empty());
    let res = w.call(&w.r1, Method::DELETE, &path, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "claim_decided");
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["status"], "pending");
    assert_eq!(res.body["decided_at"], Value::Null);
    assert_eq!(w.inbox().await.len(), 1);
}

#[tokio::test]
async fn approved_then_unlinked_can_claim_again() {
    let w = world().await;
    let made = w.claim(&w.r1, 0).await;
    w.approve(&w.ana, made.body["id"].as_str().unwrap()).await;
    w.call(
        &w.ana,
        Method::DELETE,
        &format!("/api/players/{}/link", w.rui),
        None,
    )
    .await;
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    assert_eq!(res.body["status"], "pending");
}

#[tokio::test]
async fn approve_declines_other_pending_claims() {
    let w = world().await;
    let c1 = w.claim(&w.r1, 0).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    w.claim(&w.r2, 0).await;
    assert_eq!(w.inbox().await.len(), 2);
    assert_eq!(w.approve(&w.ana, &c1).await.status, StatusCode::NO_CONTENT);
    let res = w.call(&w.r2, Method::GET, "/api/me/claims", None).await;
    assert_eq!(res.body["items"][0]["status"], "declined");
    assert!(res.body["items"][0]["decided_at"].as_i64().is_some());
    assert!(w.inbox().await.is_empty());
}

#[tokio::test]
async fn approve_after_qr_link_is_409() {
    let w = world().await;
    let c1 = w.claim(&w.r1, 0).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let made = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-code", w.rui),
            None,
        )
        .await;
    let code = support::fragment_code(made.body["url"].as_str().unwrap());
    let res = w
        .call(
            &w.r2,
            Method::POST,
            "/api/link-codes/accept",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = w.approve(&w.ana, &c1).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "player_linked");
    // Still pending, so the coordinator can decline it by id, though the linked entry has left
    // the inbox.
    assert_eq!(w.inbox().await.len(), 0);
    assert_eq!(w.decline(&w.ana, &c1).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        w.state.store.links(&w.ana_id).await.unwrap()[&w.rui].user_id,
        w.r2_id
    );
}

#[tokio::test]
async fn approve_other_coordinators_claim_is_404() {
    let w = world().await;
    let id = w.claim(&w.r1, 0).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for res in [w.approve(&w.r2, &id).await, w.decline(&w.r2, &id).await] {
        assert_eq!(res.status, StatusCode::NOT_FOUND);
    }
    let res = w.approve(&w.ana, "no-such-claim").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(w.inbox().await.len(), 1);
    assert!(w.state.store.links(&w.ana_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn pending_claims_are_capped() {
    let w = world().await;
    // Twenty pending claims, seeded directly: the rate limit would stop twenty-one API calls.
    for i in 0..20 {
        let id = w
            .state
            .store
            .create_or_get_player(&w.ana_id, &format!("P{i}"), 1)
            .await
            .unwrap()
            .0
            .id;
        sqlx::query(
            "INSERT INTO player_claims (id, player_id, claimant_user_id, owner_user_id, status, \
             created_at) VALUES (?, ?, ?, ?, 'pending', 1)",
        )
        .bind(format!("claim-{i}"))
        .bind(&id)
        .bind(&w.r1_id)
        .bind(&w.ana_id)
        .execute(&w.pool)
        .await
        .unwrap();
    }
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "too_many_claims");
    // Another account is not affected.
    assert_eq!(w.claim(&w.r2, 0).await.status, StatusCode::CREATED);
    // A withdrawn claim frees a place.
    let res = w
        .call(&w.r1, Method::DELETE, "/api/me/claims/claim-0", None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(w.claim(&w.r1, 0).await.status, StatusCode::CREATED);
}

#[tokio::test]
async fn claims_need_session() {
    let w = world().await;
    let token = device_token(&w.state.store, &w.ana_id).await;
    let routes = [
        (Method::GET, format!("/api/view/{}/slots", w.token)),
        (Method::POST, "/api/claims/preview".to_owned()),
        (Method::POST, "/api/claims".to_owned()),
        (Method::GET, "/api/claims".to_owned()),
        (Method::POST, "/api/claims/x/approve".to_owned()),
        (Method::POST, "/api/claims/x/decline".to_owned()),
        (Method::GET, "/api/me/claims".to_owned()),
        (Method::DELETE, "/api/me/claims/x".to_owned()),
        (Method::GET, "/api/me/links".to_owned()),
        (Method::DELETE, "/api/me/links/x".to_owned()),
    ];
    for (method, path) in routes {
        let body = Some(w.target(0));
        let none = send(&w.app, req(method.clone(), &path, body.clone(), &[])).await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED, "{method} {path}");
        let device = send(&w.app, as_device(&token, method.clone(), &path, body)).await;
        assert_eq!(device.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn disabled_claimant_drops_from_inbox() {
    let w = world().await;
    w.claim(&w.r1, 0).await;
    w.claim(&w.r2, 1).await;
    assert_eq!(w.inbox().await.len(), 2);
    sqlx::query("UPDATE users SET disabled_at = 5 WHERE id = ?")
        .bind(&w.r1_id)
        .execute(&w.pool)
        .await
        .unwrap();
    let inbox = w.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["claimant"]["email"], "r2@example.com");
    let players = w.player_list().await;
    let rui = players.iter().find(|p| p["id"] == w.rui.as_str()).unwrap();
    assert_eq!(rui["pending_claims"], 0);
    let sam = players.iter().find(|p| p["id"] == w.sam.as_str()).unwrap();
    assert_eq!(sam["pending_claims"], 1);
}

#[tokio::test]
async fn claim_routes_are_rate_limited() {
    let w = world().await;
    for i in 0..20 {
        let res = w
            .call(
                &w.r1,
                Method::POST,
                "/api/claims/preview",
                Some(w.target(0)),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "call {i}");
    }
    let res = w.claim(&w.r1, 0).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn preview_of_linked_entry_is_404() {
    let w = world().await;
    let preview = |who: String| {
        let w = &w;
        async move {
            w.call(&who, Method::POST, "/api/claims/preview", Some(w.target(0)))
                .await
        }
    };
    assert_eq!(preview(w.r1.clone()).await.status, StatusCode::OK);
    let res = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-me", w.rui),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = preview(w.r1.clone()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"], "not_found");
}

#[tokio::test]
async fn claims_on_linked_entries_leave_inbox_and_badge_until_unlinked() {
    let w = world().await;
    w.claim(&w.r1, 0).await;
    assert_eq!(w.inbox().await.len(), 1);
    // A QR link by another account settles the entry.
    let made = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-code", w.rui),
            None,
        )
        .await;
    let code = support::fragment_code(made.body["url"].as_str().unwrap());
    let res = w
        .call(
            &w.r2,
            Method::POST,
            "/api/link-codes/accept",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert!(w.inbox().await.is_empty());
    let badge = |list: Vec<Value>| {
        list.iter().find(|p| p["id"] == w.rui.as_str()).unwrap()["pending_claims"].clone()
    };
    assert_eq!(badge(w.player_list().await), 0);
    // Unlinked again: the pending claim is back.
    let res = w
        .call(
            &w.ana,
            Method::DELETE,
            &format!("/api/players/{}/link", w.rui),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(w.inbox().await.len(), 1);
    assert_eq!(badge(w.player_list().await), 1);
}

#[tokio::test]
async fn own_pending_claim_is_approved_when_the_claimant_accepts_a_qr() {
    let w = world().await;
    let id = w.claim(&w.r1, 0).await.body["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let made = w
        .call(
            &w.ana,
            Method::POST,
            &format!("/api/players/{}/link-code", w.rui),
            None,
        )
        .await;
    let code = support::fragment_code(made.body["url"].as_str().unwrap());
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/link-codes/accept",
            Some(json!({ "code": code })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let mine = w.call(&w.r1, Method::GET, "/api/me/claims", None).await;
    let claim = &mine.body["items"][0];
    assert_eq!(claim["id"], id.as_str());
    assert_eq!(claim["status"], "approved");
    assert!(claim["decided_at"].as_i64().is_some());
}

/// Ana's event with teams (Rui, Sam) and (Bia, Cat): `(event view token, [team ids])`.
async fn event_world(w: &World) -> (String, [String; 2]) {
    let store = &w.state.store;
    let ev = store
        .create_competition(
            &w.ana_id,
            NewCompetition {
                name: "Sunday cup".into(),
                kind: Kind::Tournament,
                format: format(),
                settings: None,
            },
            1,
        )
        .await
        .unwrap();
    let bia = store
        .create_or_get_player(&w.ana_id, "Bia", 1)
        .await
        .unwrap()
        .0
        .id;
    let cat = store
        .create_or_get_player(&w.ana_id, "Cat", 1)
        .await
        .unwrap()
        .0
        .id;
    let t1 = store
        .add_team(&w.ana_id, &ev.id, [w.rui.clone(), w.sam.clone()], None, 1)
        .await
        .unwrap();
    let t2 = store
        .add_team(&w.ana_id, &ev.id, [bia, cat], None, 1)
        .await
        .unwrap();
    (ev.spectator_token, [t1.id, t2.id])
}

fn event_target(token: &str, team_id: &str, position: u8) -> Value {
    json!({ "kind": "event_team_slot", "event_view_token": token, "team_id": team_id,
            "position": position })
}

#[tokio::test]
async fn event_slot_claim_round_trip() {
    let w = world().await;
    let (token, [t1, t2]) = event_world(&w).await;
    let slots_path = format!("/api/event-view/{token}/slots");

    let res = w.call(&w.r1, Method::GET, &slots_path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body["items"],
        json!([
            { "team_id": t1, "position": 0, "name": "Rui", "state": "claimable" },
            { "team_id": t1, "position": 1, "name": "Sam", "state": "claimable" },
            { "team_id": t2, "position": 0, "name": "Bia", "state": "claimable" },
            { "team_id": t2, "position": 1, "name": "Cat", "state": "claimable" },
        ])
    );

    // Sam's preview counts the event, which has no placing yet.
    let res = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims/preview",
            Some(event_target(&token, &t1, 1)),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["name"], "Sam");
    assert_eq!(res.body["events"], 1);

    let made = w
        .call(
            &w.r1,
            Method::POST,
            "/api/claims",
            Some(event_target(&token, &t1, 1)),
        )
        .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let res = w.call(&w.r1, Method::GET, &slots_path, None).await;
    assert_eq!(res.body["items"][1]["state"], "pending");

    let inbox = w.inbox().await;
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["player"], json!({ "id": w.sam, "name": "Sam" }));
    let res = w.approve(&w.ana, inbox[0]["id"].as_str().unwrap()).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);

    let links = w.call(&w.r1, Method::GET, "/api/me/links", None).await;
    assert_eq!(links.body["items"][0]["player_id"], w.sam.as_str());
    let res = w.call(&w.r1, Method::GET, &slots_path, None).await;
    assert_eq!(res.body["items"][1]["state"], "yours");
    let res = w.call(&w.r2, Method::GET, &slots_path, None).await;
    assert_eq!(res.body["items"][1]["state"], "linked");
    assert_eq!(res.body["items"][0]["state"], "claimable");
}

#[tokio::test]
async fn event_slot_targets_that_do_not_resolve_are_404() {
    let w = world().await;
    let (token, [t1, _]) = event_world(&w).await;
    // A team of another event: the target must name the event the team is in.
    let (other_token, [other_team, _]) = event_world(&w).await;
    let bad = [
        event_target("no-such-token", &t1, 0),
        event_target(&token, &other_team, 0),
        event_target(&other_token, &t1, 0),
        event_target(&token, &t1, 2),
        event_target(&token, "no-such-team", 0),
    ];
    for target in bad {
        for path in ["/api/claims/preview", "/api/claims"] {
            let res = w
                .call(&w.r1, Method::POST, path, Some(target.clone()))
                .await;
            assert_eq!(res.status, StatusCode::NOT_FOUND, "{path} {target}");
        }
    }
    let res = w
        .call(&w.r1, Method::GET, "/api/event-view/no-such/slots", None)
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // A scoring or court token is not an event view token.
    let res = w
        .call(
            &w.r1,
            Method::GET,
            &format!("/api/event-view/{}/slots", w.token),
            None,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}
