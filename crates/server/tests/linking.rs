#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use padel_score_server::accounts_store::NewUser;
use padel_score_server::db;
use padel_score_server::linking::{ClaimPreview, preview_in};
use padel_score_server::players::PlayerSlots;
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use sqlx::SqlitePool;

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
