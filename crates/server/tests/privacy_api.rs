#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::collections::HashSet;

use axum::Router;
use axum::http::{Method, StatusCode};
use padel_score_server::AppState;
use padel_score_server::api::now;
use padel_score_server::players::PlayerSlots;
use padel_score_server::store::{NewMatch, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Res, as_session, invited_user, req, send, with_admin};

const SCORE: &str =
    r#"{"sets":[],"games":[6,3],"regulation_games":[6,3],"overtime":false,"timed":false}"#;

/// Ana (admin) and Bea coordinate; R is the account linked to Ana's "Rui" and Bea's "Rui C.".
struct World {
    state: AppState,
    pool: SqlitePool,
    app: Router,
    ana_id: String,
    ana: String,
    bea_id: String,
    bea: String,
    r_id: String,
    r: String,
    rui: String,
    rui_c: String,
    /// Ana's two matches (finished 1000 and 3000) and Bea's (2000), which is "Match 1" of Bea's
    /// event `ev1`, "Bea's cup"; Rui C. placed 2nd of 8 in it.
    a1: String,
    a2: String,
    b1: String,
}

fn format() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn finished(
    state: &AppState,
    pool: &SqlitePool,
    owner: &str,
    a: [Option<&str>; 2],
    b: [Option<&str>; 2],
    winner: Option<&str>,
    at: i64,
) -> String {
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
                    a: ["Ona".into(), "Pia".into()],
                    b: ["Tom".into(), "Una".into()],
                },
                owner_user_id: Some(owner.to_owned()),
                players: PlayerSlots {
                    a: slot(a),
                    b: slot(b),
                },
            },
            1,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO match_results (match_id, owner_user_id, winner, score, finished_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&m.id)
    .bind(owner)
    .bind(winner)
    .bind(SCORE)
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
    m.id
}

async fn player(state: &AppState, owner: &str, name: &str) -> String {
    state
        .store
        .create_or_get_player(owner, name, 1)
        .await
        .unwrap()
        .0
        .id
}

/// Make Bea's match `b1` "Match 1" of her event `ev1`, "Bea's cup", in which Rui C. plays.
async fn bea_event(state: &AppState, pool: &SqlitePool, bea_id: &str, rui_c: &str, b1: &str) {
    let partner = player(state, bea_id, "Pia B.").await;
    let tom = player(state, bea_id, "Tom B.").await;
    let una = player(state, bea_id, "Una B.").await;
    sqlx::query(
        "INSERT INTO competitions (id, owner_user_id, name, kind, format, status, \
         spectator_token, created_at, started_at, finished_at) \
         VALUES ('ev1', ?, 'Bea''s cup', 'tournament', '{}', 'finished', 'ev1-view', 1, 1500, \
         2500)",
    )
    .bind(bea_id)
    .execute(pool)
    .await
    .unwrap();
    for (id, p1, p2, pos) in [("t1", rui_c, &*partner, 0), ("t2", &*tom, &*una, 1)] {
        sqlx::query(
            "INSERT INTO competition_teams (id, event_id, player1_id, player2_id, position) \
             VALUES (?, 'ev1', ?, ?, ?)",
        )
        .bind(id)
        .bind(p1)
        .bind(p2)
        .bind(pos)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO competition_matches (match_id, event_id, team_a_id, team_b_id, label, \
         ordinal) VALUES (?, 'ev1', 't1', 't2', 'Match 1', 1)",
    )
    .bind(b1)
    .execute(pool)
    .await
    .unwrap();
}

/// Not yet linked: Rui (Ana) won at 1000 and lost at 3000; Rui C. (Bea) won at 2000.
async fn world() -> World {
    let (state, pool, app, ana_id, ana) = with_admin().await;
    let (bea_id, bea) = invited_user(&state, &app, &ana, "bea@example.com", "Bea").await;
    let (r_id, r) = invited_user(&state, &app, &ana, "r@example.com", "Rui Costa").await;
    let rui = player(&state, &ana_id, "Rui").await;
    let sam = player(&state, &ana_id, "Sam").await;
    let rui_c = player(&state, &bea_id, "Rui C.").await;
    let a1 = finished(
        &state,
        &pool,
        &ana_id,
        [Some(&rui), Some(&sam)],
        [None, None],
        Some("a"),
        1000,
    )
    .await;
    let a2 = finished(
        &state,
        &pool,
        &ana_id,
        [Some(&sam), None],
        [Some(&rui), None],
        Some("a"),
        3000,
    )
    .await;
    let b1 = finished(
        &state,
        &pool,
        &bea_id,
        [Some(&rui_c), None],
        [None, None],
        Some("a"),
        2000,
    )
    .await;
    bea_event(&state, &pool, &bea_id, &rui_c, &b1).await;
    sqlx::query(
        "INSERT INTO placings (player_id, event_id, event_kind, place, place_to, field_size, \
         finished_at) VALUES (?, 'ev1', 'tournament', 2, 2, 8, 2500)",
    )
    .bind(&rui)
    .execute(&pool)
    .await
    .unwrap();
    World {
        state,
        pool,
        app,
        ana_id,
        ana,
        bea_id,
        bea,
        r_id,
        r,
        rui,
        rui_c,
        a1,
        a2,
        b1,
    }
}

impl World {
    async fn get(&self, who: &str, path: &str) -> Res {
        send(
            &self.app,
            as_session(&self.state, who, Method::GET, path, None),
        )
        .await
    }

    async fn link(&self, owner: &str, player: &str, user: &str) {
        let (code, _) = self
            .state
            .store
            .create_link_code(owner, player, now())
            .await
            .unwrap();
        self.state
            .store
            .accept_link_code(&code, user, now())
            .await
            .unwrap()
            .unwrap();
    }

    async fn link_both(&self) {
        self.link(&self.ana_id, &self.rui, &self.r_id).await;
        self.link(&self.bea_id, &self.rui_c, &self.r_id).await;
    }

    async fn set_visibility(&self, who: &str, v: &str) -> Res {
        send(
            &self.app,
            as_session(
                &self.state,
                who,
                Method::PUT,
                "/api/me/privacy",
                Some(json!({ "visibility": v })),
            ),
        )
        .await
    }

    async fn profile(&self, token: &str) -> Res {
        send(
            &self.app,
            req(Method::GET, &format!("/api/profiles/{token}"), None, &[]),
        )
        .await
    }

    async fn profile_token(&self) -> String {
        let res = self.set_visibility(&self.r, "public").await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body["profile_url"]
            .as_str()
            .unwrap()
            .strip_prefix("/p/")
            .unwrap()
            .to_owned()
    }
}

fn ids(res: &Res) -> Vec<&str> {
    res.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["match_id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn claimed_history_moves_to_player() {
    let w = world().await;
    let before = w.get(&w.r, "/api/me/results").await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.body);
    assert_eq!(before.body["items"], json!([]));
    assert_eq!(
        before.body["record"],
        json!({"wins": 0, "losses": 0, "draws": 0})
    );
    assert_eq!(before.body["next"], Value::Null);

    w.link_both().await;
    let res = w.get(&w.r, "/api/me/results").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(ids(&res), [&w.a2, &w.b1, &w.a1]);
    let items = res.body["items"].as_array().unwrap();
    // a2: Rui on team b, team a won; b1: win on a; a1: win on a.
    assert_eq!(items[0]["outcome"], "loss");
    assert_eq!(items[0]["side"], "b");
    assert_eq!(items[1]["outcome"], "win");
    assert_eq!(items[2]["outcome"], "win");
    assert_eq!(items[0]["coordinator_name"], "Ana");
    assert_eq!(items[1]["coordinator_name"], "Bea");
    assert_eq!(items[1]["redacted"], false);
    assert_eq!(items[1]["teams"]["a"], json!(["Rui C.", "Pia"]));
    assert!(items[1]["spectator_token"].is_string());
    assert_eq!(items[0]["score"]["games"], json!([6, 3]));
    assert_eq!(res.body["display_name"], "Rui Costa");
    assert_eq!(
        res.body["record"],
        json!({"wins": 2, "losses": 1, "draws": 0})
    );
    let placings = res.body["placings"].as_array().unwrap();
    assert_eq!(placings.len(), 1);
    assert_eq!(placings[0]["event_id"], "ev1");
    assert_eq!(res.body["next"], Value::Null);
}

#[tokio::test]
async fn unlink_moves_history_back() {
    let w = world().await;
    w.link_both().await;
    let path = format!("/api/players/{}/link", w.rui_c);
    let res = send(
        &w.app,
        as_session(&w.state, &w.bea, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = w.get(&w.r, "/api/me/results").await;
    assert_eq!(ids(&res), [&w.a2, &w.a1]);
    assert_eq!(
        res.body["record"],
        json!({"wins": 1, "losses": 1, "draws": 0})
    );
}

#[tokio::test]
async fn coordinator_sees_own_matches_full_and_others_redacted() {
    let w = world().await;
    w.link_both().await;
    let path = format!("/api/players/{}/linked-history", w.rui);
    let res = w.get(&w.ana, &path).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(ids(&res), [&w.a2, &w.b1, &w.a1]);
    let items = res.body["items"].as_array().unwrap();
    for i in [0, 2] {
        assert_eq!(items[i]["redacted"], false);
        assert!(items[i]["teams"].is_object());
        assert_eq!(items[i]["coordinator_name"], "Ana");
        assert!(items[i]["spectator_token"].is_string());
    }
    let redacted = items[1].as_object().unwrap();
    assert_eq!(redacted["redacted"], true);
    for key in ["teams", "coordinator_name", "spectator_token"] {
        assert!(!redacted.contains_key(key), "{key} leaked");
    }
    assert_eq!(redacted["outcome"], "win");
    assert_eq!(res.body["display_name"], "Rui Costa");
    assert_eq!(
        res.body["record"],
        json!({"wins": 2, "losses": 1, "draws": 0})
    );
}

#[tokio::test]
async fn private_hides_linked_history_from_coordinator() {
    let w = world().await;
    w.link_both().await;
    let path = format!("/api/players/{}/linked-history", w.rui);
    let before = w.get(&w.ana, &format!("/api/players/{}", w.rui)).await;
    assert_eq!(
        w.set_visibility(&w.r, "private").await.status,
        StatusCode::OK
    );
    let res = w.get(&w.ana, &path).await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(res.body, json!({"error": "history_private"}));
    let after = w.get(&w.ana, &format!("/api/players/{}", w.rui)).await;
    assert_eq!(after.status, StatusCode::OK);
    assert_eq!(after.body["items"], before.body["items"]);
    assert_eq!(
        after.body["player"]["record"],
        before.body["player"]["record"]
    );
    // The player still sees their own.
    let own = w.get(&w.r, "/api/me/results").await;
    assert_eq!(own.body["items"].as_array().unwrap().len(), 3);
}

/// Ana's player page with the link fields removed.
async fn page_without_link(w: &World) -> Value {
    let mut body = w.get(&w.ana, &format!("/api/players/{}", w.rui)).await.body;
    let player = body["player"].as_object_mut().unwrap();
    player.remove("link");
    player.remove("pending_claims");
    body
}

#[tokio::test]
async fn coordinator_view_unchanged_by_link_and_privacy() {
    let w = world().await;
    let base = page_without_link(&w).await;
    assert_eq!(base["items"].as_array().unwrap().len(), 2);
    w.link_both().await;
    assert_eq!(page_without_link(&w).await, base);
    w.set_visibility(&w.r, "private").await;
    assert_eq!(page_without_link(&w).await, base);
    w.set_visibility(&w.r, "public").await;
    assert_eq!(page_without_link(&w).await, base);
    let path = format!("/api/players/{}/link", w.rui);
    send(
        &w.app,
        as_session(&w.state, &w.ana, Method::DELETE, &path, None),
    )
    .await;
    assert_eq!(page_without_link(&w).await, base);
}

#[tokio::test]
async fn public_profile_is_redacted() {
    let w = world().await;
    w.link_both().await;
    let token = w.profile_token().await;
    let res = w.profile(&token).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["display_name"], "Rui Costa");
    assert_eq!(
        res.body["record"],
        json!({"wins": 2, "losses": 1, "draws": 0})
    );
    assert_eq!(res.body["placings"].as_array().unwrap().len(), 1);
    let items = res.body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for item in items {
        let o = item.as_object().unwrap();
        assert_eq!(o["redacted"], true);
        for key in ["teams", "coordinator_name", "spectator_token"] {
            assert!(!o.contains_key(key), "{key} leaked");
        }
    }
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
    assert_eq!(
        res.header(axum::http::header::REFERRER_POLICY),
        Some("no-referrer")
    );
}

#[tokio::test]
async fn profile_404_after_leaving_public() {
    let w = world().await;
    w.link_both().await;
    let token = w.profile_token().await;
    assert_eq!(w.profile(&token).await.status, StatusCode::OK);
    for v in ["coordinators", "private"] {
        w.set_visibility(&w.r, v).await;
        assert_eq!(w.profile(&token).await.status, StatusCode::NOT_FOUND);
        let privacy = w.get(&w.r, "/api/me/privacy").await;
        assert_eq!(privacy.body, json!({"visibility": v, "profile_url": null}));
        // Going public again gives a different link; the old one stays dead.
        let again = w.profile_token().await;
        assert_ne!(again, token);
        assert_eq!(w.profile(&token).await.status, StatusCode::NOT_FOUND);
        assert_eq!(w.profile(&again).await.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn rotated_profile_token_old_is_404() {
    let w = world().await;
    let token = w.profile_token().await;
    let res = send(
        &w.app,
        as_session(&w.state, &w.r, Method::POST, "/api/me/privacy/rotate", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["visibility"], "public");
    let new = res.body["profile_url"]
        .as_str()
        .unwrap()
        .strip_prefix("/p/")
        .unwrap()
        .to_owned();
    assert_ne!(new, token);
    assert_eq!(w.profile(&token).await.status, StatusCode::NOT_FOUND);
    assert_eq!(w.profile(&new).await.status, StatusCode::OK);
    // Not public: 409.
    w.set_visibility(&w.r, "private").await;
    let res = send(
        &w.app,
        as_session(&w.state, &w.r, Method::POST, "/api/me/privacy/rotate", None),
    )
    .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({"error": "not_public"}));
}

#[tokio::test]
async fn unknown_profile_token_is_404() {
    let w = world().await;
    let res = w.profile("nope").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body, json!({"error": "not_found"}));
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
}

#[tokio::test]
async fn default_visibility_is_coordinators() {
    let w = world().await;
    let res = w.get(&w.r, "/api/me/privacy").await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.body,
        json!({"visibility": "coordinators", "profile_url": null})
    );
}

#[tokio::test]
async fn bad_visibility_is_422() {
    let w = world().await;
    let want = json!({"error": "validation", "field": "visibility"});
    for body in [
        json!({"visibility": "everyone"}),
        json!({}),
        json!({"visibility": 3}),
    ] {
        let res = send(
            &w.app,
            as_session(&w.state, &w.r, Method::PUT, "/api/me/privacy", Some(body)),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.body);
        assert_eq!(res.body, want);
    }
}

#[tokio::test]
async fn linked_history_needs_owner() {
    let w = world().await;
    w.link_both().await;
    let path = format!("/api/players/{}/linked-history", w.rui);
    assert_eq!(w.get(&w.bea, &path).await.status, StatusCode::NOT_FOUND);
    // Unlinked entry of the caller: 409.
    let sam = player(&w.state, &w.ana_id, "Sam2").await;
    let res = w
        .get(&w.ana, &format!("/api/players/{sam}/linked-history"))
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({"error": "player_not_linked"}));
}

#[tokio::test]
async fn results_pagination_ties_on_finished_at() {
    let w = world().await;
    let mut want = HashSet::new();
    for _ in 0..3 {
        want.insert(
            finished(
                &w.state,
                &w.pool,
                &w.ana_id,
                [Some(&w.rui), None],
                [None, None],
                Some("a"),
                9000,
            )
            .await,
        );
    }
    for _ in 0..2 {
        want.insert(
            finished(
                &w.state,
                &w.pool,
                &w.bea_id,
                [Some(&w.rui_c), None],
                [None, None],
                Some("a"),
                9000,
            )
            .await,
        );
    }
    w.link_both().await;
    let mut seen = Vec::new();
    let mut path = "/api/me/results?limit=2".to_owned();
    loop {
        let res = w.get(&w.r, &path).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert!(res.body["items"].as_array().unwrap().len() <= 2);
        seen.extend(ids(&res).into_iter().map(str::to_owned));
        match res.body["next"].as_str() {
            Some(next) => path = format!("/api/me/results?limit=2&before={next}"),
            None => break,
        }
    }
    let tied: Vec<_> = seen.iter().filter(|id| want.contains(*id)).collect();
    assert_eq!(tied.len(), 5);
    assert_eq!(seen.len(), 8);
    assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 8);
}

#[tokio::test]
async fn privacy_routes_need_session() {
    let w = world().await;
    let routes = [
        (Method::GET, "/api/me/results"),
        (Method::GET, "/api/me/privacy"),
        (Method::PUT, "/api/me/privacy"),
        (Method::POST, "/api/me/privacy/rotate"),
        (Method::GET, "/api/players/anyone/linked-history"),
    ];
    let token = support::device_token(&w.state.store, &w.r_id).await;
    for (m, p) in routes {
        assert_eq!(
            send(&w.app, req(m.clone(), p, None, &[])).await.status,
            StatusCode::UNAUTHORIZED,
            "{m} {p}"
        );
        let res = send(&w.app, support::as_device(&token, m.clone(), p, None)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{m} {p}");
    }
}
