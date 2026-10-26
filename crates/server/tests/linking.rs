#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::http::{Method, StatusCode};
use padel_score_server::accounts_store::NewUser;
use padel_score_server::competitions::{Kind, NewCompetition, TeamChange};
use padel_score_server::db;
use padel_score_server::linking::{ClaimPreview, preview_in};
use padel_score_server::players::PlayerSlots;
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use serde_json::json;
use sqlx::SqlitePool;
use support::{
    as_device, as_session, device_token, fragment_code, invited_user, req, send, with_admin,
};

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

async fn player(s: &Store, owner: &str, name: &str) -> String {
    s.create_or_get_player(owner, name, 1).await.unwrap().0.id
}

fn format() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

/// A new match with the given picked players on team a (up to two) and b (up to two).
async fn play(s: &Store, owner: &str, a: [Option<&str>; 2], b: [Option<&str>; 2]) -> String {
    let slot = |ids: [Option<&str>; 2]| ids.map(|id| id.map(str::to_owned));
    s.create_match(
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
            owner_user_id: Some(owner.to_owned()),
            players: PlayerSlots {
                a: slot(a),
                b: slot(b),
            },
        },
        1,
    )
    .await
    .unwrap()
    .id
}

/// The pool holds one connection, so take it only for the call.
async fn preview(pool: &SqlitePool, player_id: &str) -> Result<ClaimPreview, StoreError> {
    let mut conn = pool.acquire().await.unwrap();
    preview_in(&mut conn, player_id).await
}

async fn finish(pool: &SqlitePool, match_id: &str, owner: &str, at: i64) {
    sqlx::query(
        "INSERT INTO match_results (match_id, owner_user_id, winner, score, finished_at) \
         VALUES (?, ?, 'a', '{}', ?)",
    )
    .bind(match_id)
    .bind(owner)
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
}

async fn placing(pool: &SqlitePool, player_id: &str, event_id: &str, at: i64) {
    sqlx::query(
        "INSERT INTO placings (player_id, event_id, event_kind, place, place_to, field_size, \
         finished_at) VALUES (?, ?, 'tournament', 1, 1, 8, ?)",
    )
    .bind(player_id)
    .bind(event_id)
    .bind(at)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_keeps_existing_rows() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = player(&s, &ana, "Rui").await;
    let linked: Option<String> =
        sqlx::query_scalar("SELECT linked_user_id FROM players WHERE id = ?")
            .bind(&rui)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(linked, None);
    let visibility: String =
        sqlx::query_scalar("SELECT history_visibility FROM users WHERE id = ?")
            .bind(&ana)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(visibility, "coordinators");
}

#[tokio::test]
async fn link_me_links_and_unlink_unlinks() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let ana_entry = player(&s, &ana, "Ana").await;
    s.link_me(&ana, &ana_entry, 50).await.unwrap();

    let links = s.links(&ana).await.unwrap();
    assert_eq!(links.len(), 1);
    let link = &links[&ana_entry];
    assert_eq!(link.user_id, ana);
    assert_eq!(link.display_name, "Ana");
    assert_eq!(link.linked_at, 50);

    let mine = s.my_links(&ana).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].player_id, ana_entry);
    assert_eq!(mine[0].name, "Ana");
    assert_eq!(mine[0].coordinator_name, "Ana");
    assert_eq!(mine[0].linked_at, 50);
    let json = serde_json::to_value(&mine[0]).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"player_id": ana_entry, "name": "Ana",
            "coordinator_name": "Ana", "linked_at": 50})
    );

    s.unlink(&ana, &ana_entry).await.unwrap();
    assert!(s.links(&ana).await.unwrap().is_empty());
    assert!(s.my_links(&ana).await.unwrap().is_empty());
    // Unlinking an unlinked entry is a no-op.
    s.unlink(&ana, &ana_entry).await.unwrap();
}

#[tokio::test]
async fn my_links_are_newest_first_across_coordinators() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let bia = user(&s, "bia@example.com", "Bia").await;
    let rui = user(&s, "rui@example.com", "Rui Costa").await;
    let old = player(&s, &ana, "Rui").await;
    let new = player(&s, &bia, "Rui C.").await;
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &old, &rui, 10)
        .await
        .unwrap();
    padel_score_server::linking::link_in(&mut tx, &new, &rui, 20)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mine = s.my_links(&rui).await.unwrap();
    let got: Vec<_> = mine
        .iter()
        .map(|l| (l.coordinator_name.as_str(), l.linked_at))
        .collect();
    assert_eq!(got, [("Bia", 20), ("Ana", 10)]);
}

#[tokio::test]
async fn link_to_other_user_is_conflict() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u1 = user(&s, "u1@example.com", "U1").await;
    let u2 = user(&s, "u2@example.com", "U2").await;
    let rui = player(&s, &ana, "Rui").await;

    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u1, 10)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut tx = s.begin().await.unwrap();
    let err = padel_score_server::linking::link_in(&mut tx, &rui, &u2, 20)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("player_linked")),
        "{err:?}"
    );
    drop(tx);

    // The same user again is a no-op, and keeps the first link time.
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u1, 30)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(s.my_links(&u1).await.unwrap()[0].linked_at, 10);
}

#[tokio::test]
async fn overlapping_entries_cannot_both_link() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u = user(&s, "u@example.com", "U").await;
    let rui = player(&s, &ana, "Rui").await;
    let rui_s = player(&s, &ana, "Rui S.").await;
    let marta = player(&s, &ana, "Marta").await;
    let joao = player(&s, &ana, "Joao").await;
    // Rui and Rui S. played one match together; Marta and Joao each played with Rui elsewhere.
    play(&s, &ana, [Some(&rui), Some(&rui_s)], [None, None]).await;
    play(&s, &ana, [Some(&marta), None], [Some(&joao), None]).await;

    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u, 10)
        .await
        .unwrap();
    let err = padel_score_server::linking::link_in(&mut tx, &rui_s, &u, 11)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("link_overlaps")),
        "{err:?}"
    );
    // Entries with no shared match both link.
    padel_score_server::linking::link_in(&mut tx, &marta, &u, 12)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(s.my_links(&u).await.unwrap().len(), 2);
}

#[tokio::test]
async fn overlapping_through_an_event_cannot_both_link() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u = user(&s, "u@example.com", "U").await;
    let rui = player(&s, &ana, "Rui").await;
    let rui_s = player(&s, &ana, "Rui S.").await;
    let other = player(&s, &ana, "Marta").await;
    placing(&pool, &rui, "ev1", 5).await;
    placing(&pool, &rui_s, "ev1", 5).await;
    placing(&pool, &other, "ev2", 5).await;
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u, 10)
        .await
        .unwrap();
    let err = padel_score_server::linking::link_in(&mut tx, &rui_s, &u, 11)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("link_overlaps")),
        "{err:?}"
    );
    padel_score_server::linking::link_in(&mut tx, &other, &u, 12)
        .await
        .unwrap();
}

/// A new event of `owner`'s with one team per pair of players: the event id.
async fn event_with_teams(s: &Store, owner: &str, teams: &[[&str; 2]]) -> String {
    let ev = s
        .create_competition(
            owner,
            NewCompetition {
                name: "Cup".into(),
                kind: Kind::Tournament,
                format: format(),
                settings: None,
            },
            1,
        )
        .await
        .unwrap();
    for [p1, p2] in teams {
        s.add_team(owner, &ev.id, [(*p1).to_owned(), (*p2).to_owned()], None, 1)
            .await
            .unwrap();
    }
    ev.id
}

#[tokio::test]
async fn overlapping_through_an_event_team_cannot_both_link() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u = user(&s, "u@example.com", "U").await;
    let rui = player(&s, &ana, "Rui").await;
    let rui_s = player(&s, &ana, "Rui S.").await;
    let marta = player(&s, &ana, "Marta").await;
    let marta_s = player(&s, &ana, "Marta S.").await;
    let x = player(&s, &ana, "X").await;
    let y = player(&s, &ana, "Y").await;
    let solo = player(&s, &ana, "Solo").await;
    // Opposing teams of one event (it has not started, so there are no placings).
    event_with_teams(&s, &ana, &[[&rui, &x], [&rui_s, &y]]).await;
    // Partners in one team of another event.
    event_with_teams(&s, &ana, &[[&marta, &marta_s]]).await;
    // A team of a third event shares nothing with the others.
    event_with_teams(&s, &ana, &[[&solo, &x]]).await;
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u, 10)
        .await
        .unwrap();
    let err = padel_score_server::linking::link_in(&mut tx, &rui_s, &u, 11)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("link_overlaps")),
        "{err:?}"
    );
    padel_score_server::linking::link_in(&mut tx, &marta, &u, 12)
        .await
        .unwrap();
    let err = padel_score_server::linking::link_in(&mut tx, &marta_s, &u, 13)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("link_overlaps")),
        "{err:?}"
    );
    padel_score_server::linking::link_in(&mut tx, &solo, &u, 14)
        .await
        .unwrap();
}

#[tokio::test]
async fn linked_entries_cannot_share_an_event() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u = user(&s, "u@example.com", "U").await;
    let v = user(&s, "v@example.com", "V").await;
    let rui = player(&s, &ana, "Rui").await;
    let rui_s = player(&s, &ana, "Rui S.").await;
    let vic = player(&s, &ana, "Vic").await;
    let x = player(&s, &ana, "X").await;
    let y = player(&s, &ana, "Y").await;
    let z = player(&s, &ana, "Z").await;
    let mut tx = s.begin().await.unwrap();
    for (p, who) in [(&rui, &u), (&rui_s, &u), (&vic, &v)] {
        padel_score_server::linking::link_in(&mut tx, p, who, 10)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let ev = event_with_teams(&s, &ana, &[]).await;
    let conflict = |r: Result<_, StoreError>| match r {
        Err(StoreError::Conflict("player_in_team")) => {}
        other => panic!("{other:?}"),
    };

    // One team: two entries of U.
    conflict(
        s.add_team(&ana, &ev, [rui.clone(), rui_s.clone()], None, 1)
            .await,
    );
    // Opposing teams: U's other entry is already in this event.
    let t1 = s
        .add_team(&ana, &ev, [rui.clone(), x.clone()], None, 1)
        .await
        .unwrap();
    conflict(
        s.add_team(&ana, &ev, [rui_s.clone(), y.clone()], None, 1)
            .await,
    );
    // Linked to different users, or unlinked: allowed.
    let t2 = s
        .add_team(&ana, &ev, [vic.clone(), y.clone()], None, 1)
        .await
        .unwrap();
    // Re-pairing: into U's other entry is refused, its own team excepted.
    conflict(
        s.update_team(
            &ana,
            &ev,
            &t2.id,
            TeamChange {
                name: None,
                players: Some([rui_s.clone(), y.clone()]),
            },
        )
        .await,
    );
    s.update_team(
        &ana,
        &ev,
        &t1.id,
        TeamChange {
            name: None,
            players: Some([rui.clone(), z.clone()]),
        },
    )
    .await
    .unwrap();
    // Another event is a different event.
    let other = event_with_teams(&s, &ana, &[]).await;
    s.add_team(&ana, &other, [rui_s.clone(), x.clone()], None, 1)
        .await
        .unwrap();
}

#[tokio::test]
async fn preview_counts_events_from_teams_once() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = player(&s, &ana, "Rui").await;
    let x = player(&s, &ana, "X").await;
    // A running event: a team, no placing yet.
    let running = event_with_teams(&s, &ana, &[[&rui, &x]]).await;
    sqlx::query("UPDATE competitions SET status = 'running', started_at = 400 WHERE id = ?")
        .bind(&running)
        .execute(&pool)
        .await
        .unwrap();
    let p = preview(&pool, &rui).await.unwrap();
    assert_eq!((p.matches, p.events), (0, 1));
    assert_eq!((p.first_at, p.last_at), (Some(400), Some(400)));

    // A finished event: its team and its placing are one event.
    let finished = event_with_teams(&s, &ana, &[[&x, &rui]]).await;
    sqlx::query(
        "UPDATE competitions SET status = 'finished', started_at = 100, finished_at = 200 \
         WHERE id = ?",
    )
    .bind(&finished)
    .execute(&pool)
    .await
    .unwrap();
    placing(&pool, &rui, &finished, 200).await;
    let p = preview(&pool, &rui).await.unwrap();
    assert_eq!((p.matches, p.events), (0, 2));
    assert_eq!((p.first_at, p.last_at), (Some(100), Some(400)));
}

#[tokio::test]
async fn two_entries_of_one_account_in_one_match_is_422() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u = user(&s, "u@example.com", "U").await;
    let rui = player(&s, &ana, "Rui").await;
    let rui_s = player(&s, &ana, "Rui S.").await;
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u, 10)
        .await
        .unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui_s, &u, 11)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let err = s
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
                owner_user_id: Some(ana.clone()),
                players: PlayerSlots {
                    a: [Some(rui.clone()), None],
                    b: [Some(rui_s.clone()), None],
                },
            },
            2,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidPlayers("same player twice")),
        "{err:?}"
    );
}

#[tokio::test]
async fn preview_counts_matches_and_dates() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui = player(&s, &ana, "Rui").await;
    let empty = player(&s, &ana, "Nobody").await;
    let m1 = play(&s, &ana, [Some(&rui), None], [None, None]).await;
    let m2 = play(&s, &ana, [None, None], [Some(&rui), None]).await;
    play(&s, &ana, [Some(&rui), None], [None, None]).await; // in progress
    finish(&pool, &m1, &ana, 100).await;
    finish(&pool, &m2, &ana, 200).await;

    let p = preview(&pool, &rui).await.unwrap();
    assert_eq!(p.name, "Rui");
    assert_eq!(p.coordinator_name, "Ana");
    assert_eq!((p.matches, p.events), (2, 0));
    assert_eq!((p.first_at, p.last_at), (Some(100), Some(200)));

    placing(&pool, &rui, "ev1", 50).await;
    placing(&pool, &rui, "ev2", 300).await;
    let p = preview(&pool, &rui).await.unwrap();
    assert_eq!((p.matches, p.events), (2, 2));
    assert_eq!((p.first_at, p.last_at), (Some(50), Some(300)));

    let p = preview(&pool, &empty).await.unwrap();
    assert_eq!(
        (p.matches, p.events, p.first_at, p.last_at),
        (0, 0, None, None)
    );

    let err = preview(&pool, "no-such").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
}

#[tokio::test]
async fn unlink_other_owners_player_is_not_found() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let bia = user(&s, "bia@example.com", "Bia").await;
    let rui = player(&s, &ana, "Rui").await;
    assert!(matches!(
        s.unlink(&bia, &rui).await.unwrap_err(),
        StoreError::NotFound
    ));
    assert!(matches!(
        s.link_me(&bia, &rui, 1).await.unwrap_err(),
        StoreError::NotFound
    ));
    assert!(matches!(
        s.unlink(&ana, "no-such").await.unwrap_err(),
        StoreError::NotFound
    ));
}

#[tokio::test]
async fn unlink_self_needs_own_link() {
    let (s, _pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let u1 = user(&s, "u1@example.com", "U1").await;
    let u2 = user(&s, "u2@example.com", "U2").await;
    let rui = player(&s, &ana, "Rui").await;
    assert!(matches!(
        s.unlink_self(&u1, &rui).await.unwrap_err(),
        StoreError::NotFound
    ));
    let mut tx = s.begin().await.unwrap();
    padel_score_server::linking::link_in(&mut tx, &rui, &u1, 10)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        s.unlink_self(&u2, &rui).await.unwrap_err(),
        StoreError::NotFound
    ));
    s.unlink_self(&u1, &rui).await.unwrap();
    assert!(s.my_links(&u1).await.unwrap().is_empty());
}

// Link codes.

use padel_score_server::AppState;
use padel_score_server::linking::LINK_CODE_TTL_SECS;

/// Ana (admin, with a player "Rui") and Rui's own account.
struct Qr {
    state: AppState,
    pool: SqlitePool,
    app: axum::Router,
    ana_id: String,
    ana: String,
    rui: String,
    rui_user: String,
    rui_session: String,
}

async fn qr_setup() -> Qr {
    let (state, pool, app, ana_id, ana) = with_admin().await;
    let (rui_user, rui_session) =
        invited_user(&state, &app, &ana, "rui@example.com", "Rui Costa").await;
    let res = send(
        &app,
        as_session(
            &state,
            &ana,
            Method::POST,
            "/api/players",
            Some(json!({ "name": "Rui" })),
        ),
    )
    .await;
    let rui = res.body["id"].as_str().unwrap().to_owned();
    Qr {
        state,
        pool,
        app,
        ana_id,
        ana,
        rui,
        rui_user,
        rui_session,
    }
}

impl Qr {
    async fn make_code(&self, session: &str, player: &str) -> support::Res {
        let path = format!("/api/players/{player}/link-code");
        send(
            &self.app,
            as_session(&self.state, session, Method::POST, &path, None),
        )
        .await
    }

    async fn post_code(&self, session: &str, route: &str, code: &str) -> support::Res {
        let body = Some(json!({ "code": code }));
        send(
            &self.app,
            as_session(&self.state, session, Method::POST, route, body),
        )
        .await
    }

    /// A fresh code for Rui's entry.
    async fn code(&self) -> String {
        let made = self.make_code(&self.ana, &self.rui).await;
        assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
        fragment_code(made.body["url"].as_str().unwrap())
    }
}

#[tokio::test]
async fn qr_link_round_trip() {
    let q = qr_setup().await;
    let made = q.make_code(&q.ana, &q.rui).await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", made.body);
    let expires_at = made.body["expires_at"].as_i64().unwrap();
    assert!(expires_at > 0);
    let expires_in = made.body["expires_in"].as_i64().unwrap();
    assert!(
        (expires_in - LINK_CODE_TTL_SECS).abs() <= 2 && expires_at > expires_in,
        "{}",
        made.body
    );
    let code = fragment_code(made.body["url"].as_str().unwrap());
    let looked = q
        .post_code(&q.rui_session, "/api/link-codes/lookup", &code)
        .await;
    assert_eq!(looked.status, StatusCode::OK, "{}", looked.body);
    assert_eq!(looked.body["name"], "Rui");
    assert_eq!(looked.body["coordinator_name"], "Ana");
    assert_eq!(looked.body["matches"], 0);
    assert_eq!(looked.body["events"], 0);
    let accepted = q
        .post_code(&q.rui_session, "/api/link-codes/accept", &code)
        .await;
    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.body);
    assert_eq!(accepted.body["player_id"], q.rui.as_str());
    let links = q.state.store.links(&q.ana_id).await.unwrap();
    assert_eq!(links[&q.rui].user_id, q.rui_user);
    for route in ["/api/link-codes/lookup", "/api/link-codes/accept"] {
        let res = q.post_code(&q.rui_session, route, &code).await;
        assert_eq!(res.status, StatusCode::GONE, "{route}");
        assert_eq!(res.body["error"], "link_code_invalid");
    }
}

#[tokio::test]
async fn link_code_needs_session() {
    let q = qr_setup().await;
    let token = device_token(&q.state.store, &q.ana_id).await;
    let routes = [
        format!("/api/players/{}/link-code", q.rui),
        "/api/link-codes/lookup".to_owned(),
        "/api/link-codes/accept".to_owned(),
    ];
    for path in &routes {
        let body = Some(json!({ "code": "x" }));
        let none = send(&q.app, req(Method::POST, path, body.clone(), &[])).await;
        assert_eq!(none.status, StatusCode::UNAUTHORIZED, "{path}");
        let device = send(&q.app, as_device(&token, Method::POST, path, body)).await;
        assert_eq!(device.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn link_code_expires() {
    let (s, pool) = setup().await;
    let ana = user(&s, "ana@example.com", "Ana").await;
    let rui_user = user(&s, "rui@example.com", "Rui Costa").await;
    let rui = player(&s, &ana, "Rui").await;
    let (code, expires_at) = s.create_link_code(&ana, &rui, 1000).await.unwrap();
    assert_eq!(expires_at, 1000 + LINK_CODE_TTL_SECS);
    assert!(
        s.link_code_preview(&code, expires_at - 1)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        s.link_code_preview(&code, expires_at)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        s.accept_link_code(&code, &rui_user, expires_at)
            .await
            .unwrap(),
        None
    );
    assert!(s.links(&ana).await.unwrap().is_empty());
    // One second earlier it still works.
    assert_eq!(
        s.accept_link_code(&code, &rui_user, expires_at - 1)
            .await
            .unwrap(),
        Some(rui.clone())
    );
    // Only the hash is stored.
    let stored: String = sqlx::query_scalar("SELECT code_hash FROM link_codes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, code);
}

#[tokio::test]
async fn new_code_voids_old_code() {
    let q = qr_setup().await;
    let old = q.code().await;
    let new = q.code().await;
    assert_ne!(old, new);
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/accept", &old)
        .await;
    assert_eq!(res.status, StatusCode::GONE);
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/accept", &new)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[tokio::test]
async fn link_code_for_other_users_player_is_404() {
    let q = qr_setup().await;
    let res = q.make_code(&q.rui_session, &q.rui).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = q.make_code(&q.ana, "no-such").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn link_code_for_linked_player_is_409() {
    let q = qr_setup().await;
    let code = q.code().await;
    q.post_code(&q.rui_session, "/api/link-codes/accept", &code)
        .await;
    let res = q.make_code(&q.ana, &q.rui).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "player_linked");
}

#[tokio::test]
async fn code_for_entry_linked_meanwhile_is_dead() {
    let q = qr_setup().await;
    let code = q.code().await;
    // Ana links the entry to herself before Rui scans.
    let res = send(
        &q.app,
        as_session(
            &q.state,
            &q.ana,
            Method::POST,
            &format!("/api/players/{}/link-me", q.rui),
            None,
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/lookup", &code)
        .await;
    assert_eq!(res.status, StatusCode::GONE);
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/accept", &code)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "player_linked");
    // The failed accept left the code unused, and the unlink makes it work again.
    let res = send(
        &q.app,
        as_session(
            &q.state,
            &q.ana,
            Method::DELETE,
            &format!("/api/players/{}/link", q.rui),
            None,
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/accept", &code)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
}

#[tokio::test]
async fn link_code_never_in_path_or_query() {
    let q = qr_setup().await;
    let made = q.make_code(&q.ana, &q.rui).await;
    let url = made.body["url"].as_str().unwrap();
    let (before, code) = url.split_once('#').unwrap();
    assert_eq!(before, "/link-player");
    assert!(!code.is_empty() && !before.contains('?'));
    assert_eq!(
        made.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
    assert_eq!(
        made.header(axum::http::header::REFERRER_POLICY),
        Some("no-referrer")
    );
}

#[tokio::test]
async fn link_code_routes_are_rate_limited() {
    let q = qr_setup().await;
    for i in 0..20 {
        let res = q
            .post_code(&q.rui_session, "/api/link-codes/lookup", "nope")
            .await;
        assert_eq!(res.status, StatusCode::GONE, "call {i}");
    }
    let res = q
        .post_code(&q.rui_session, "/api/link-codes/lookup", "nope")
        .await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
    // Per user: another account is not affected.
    let res = q.post_code(&q.ana, "/api/link-codes/lookup", "nope").await;
    assert_eq!(res.status, StatusCode::GONE);
}

#[tokio::test]
async fn link_code_creation_is_rate_limited() {
    let q = qr_setup().await;
    for i in 0..20 {
        let res = q.make_code(&q.ana, &q.rui).await;
        assert_eq!(res.status, StatusCode::CREATED, "call {i}");
    }
    let res = q.make_code(&q.ana, &q.rui).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn link_me_and_unlink_routes() {
    let q = qr_setup().await;
    let link = format!("/api/players/{}/link", q.rui);
    let res = send(
        &q.app,
        as_session(&q.state, &q.rui_session, Method::DELETE, &link, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = send(
        &q.app,
        as_session(
            &q.state,
            &q.rui_session,
            Method::POST,
            &format!("/api/players/{}/link-me", q.rui),
            None,
        ),
    )
    .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let code = q.code().await;
    q.post_code(&q.rui_session, "/api/link-codes/accept", &code)
        .await;
    let res = send(
        &q.app,
        as_session(&q.state, &q.ana, Method::DELETE, &link, None),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(q.state.store.links(&q.ana_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn expired_link_codes_are_purged() {
    let q = qr_setup().await;
    q.code().await;
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM link_codes")
            .fetch_one(&q.pool)
            .await
            .unwrap()
    };
    let expires_at: i64 = sqlx::query_scalar("SELECT expires_at FROM link_codes")
        .fetch_one(&q.pool)
        .await
        .unwrap();
    assert_eq!(
        q.state
            .store
            .purge_expired_auth(expires_at - 1)
            .await
            .unwrap()
            .link_codes,
        0
    );
    assert_eq!(count().await, 1);
    let purged = q.state.store.purge_expired_auth(expires_at).await.unwrap();
    assert_eq!(purged.link_codes, 1);
    assert_eq!(purged.total(), 1);
    assert_eq!(count().await, 0);
}
