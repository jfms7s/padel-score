#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::HashSet;

use padel_score_server::db;
use padel_score_server::store::{
    EventState, NewMatch, Store, StoreError, Teams, append, mark, save_result, touch,
};
use padel_score_server::tokens::new_token;
use padel_scoring::{
    DecidingSet, DeuceRule, Event, Format, Player, SetRules, SetsFormat, Start, Team,
};

async fn store() -> Store {
    Store::new(db::connect("sqlite::memory:").await.unwrap())
}

fn sets_format() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::BestOf3,
        deciding: DecidingSet::FullSet,
    })
}

fn start() -> Start {
    Start {
        first_server: Player {
            team: Team::A,
            index: 0,
        },
        other_first_server: 0,
    }
}

fn teams() -> Teams {
    Teams {
        a: ["Ana".into(), "Bea".into()],
        b: ["Cris".into(), "Dani".into()],
    }
}

fn new_match() -> NewMatch {
    NewMatch {
        format: sets_format(),
        start: start(),
        teams: teams(),
    }
}

#[tokio::test]
async fn create_and_load_by_either_token() {
    let s = store().await;
    let m = s.create_match(new_match(), 1000).await.unwrap();
    assert_eq!(m.revision, 0);
    assert_eq!(m.last_activity_at, 1000);
    assert_eq!(m.teams, teams());
    let by_scorer = s.by_scorer_token(&m.scorer_token).await.unwrap().unwrap();
    assert_eq!(by_scorer, m);
    let by_spectator = s
        .by_spectator_token(&m.spectator_token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_spectator, m);
    assert!(
        s.by_scorer_token(&m.spectator_token)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        s.by_spectator_token(&m.scorer_token)
            .await
            .unwrap()
            .is_none()
    );
    assert!(s.by_scorer_token("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn tokens_are_unique_and_43_chars() {
    let tokens: HashSet<String> = (0..200).map(|_| new_token()).collect();
    assert_eq!(tokens.len(), 200);
    for t in &tokens {
        assert_eq!(t.len(), 43);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    assert_ne!(m.scorer_token, m.spectator_token);
    assert_eq!(m.scorer_token.len(), 43);
    assert_eq!(m.spectator_token.len(), 43);
}

#[tokio::test]
async fn invalid_rules_are_refused() {
    let s = store().await;
    let mut nm = new_match();
    nm.format = Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::SuperTieBreak,
    });
    assert!(matches!(
        s.create_match(nm, 1).await,
        Err(StoreError::InvalidRules(_))
    ));
    let mut nm = new_match();
    nm.start.other_first_server = 2;
    assert!(matches!(
        s.create_match(nm, 1).await,
        Err(StoreError::InvalidRules(_))
    ));
}

#[tokio::test]
async fn blank_names_get_placeholders() {
    let s = store().await;
    let mut nm = new_match();
    nm.teams = Teams {
        a: ["  ".into(), " Bea ".into()],
        b: ["Cris".into(), "".into()],
    };
    let m = s.create_match(nm, 1).await.unwrap();
    assert_eq!(m.teams.a, ["Player 1".to_string(), "Bea".to_string()]);
    assert_eq!(m.teams.b, ["Cris".to_string(), "Player 2".to_string()]);
}

#[tokio::test]
async fn long_names_are_refused_by_chars_not_bytes() {
    let s = store().await;
    let mut nm = new_match();
    nm.teams.a[0] = "é".repeat(40);
    assert!(s.create_match(nm, 1).await.is_ok());
    let mut nm = new_match();
    nm.teams.a[0] = "x".repeat(41);
    assert!(matches!(
        s.create_match(nm, 1).await,
        Err(StoreError::InvalidTeams)
    ));
}

#[tokio::test]
async fn events_round_trip_in_seq_order() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    assert_eq!(s.head_seq(&m.id).await.unwrap(), 0);
    let mut tx = s.begin().await.unwrap();
    append(&mut tx, &m.id, 2, Event::TimeUp, "c2", 11)
        .await
        .unwrap();
    append(&mut tx, &m.id, 1, Event::Point(Team::A), "c1", 10)
        .await
        .unwrap();
    append(&mut tx, &m.id, 3, Event::Point(Team::B), "c3", 12)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let evs = s.events(&m.id).await.unwrap();
    let got: Vec<(i64, Event, &str, EventState)> = evs
        .iter()
        .map(|e| (e.seq, e.event, e.client_id.as_str(), e.state))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, Event::Point(Team::A), "c1", EventState::Effective),
            (2, Event::TimeUp, "c2", EventState::Effective),
            (3, Event::Point(Team::B), "c3", EventState::Effective),
        ]
    );
    assert_eq!(s.head_seq(&m.id).await.unwrap(), 3);
}

#[tokio::test]
async fn marked_events_keep_their_rows() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    for seq in 1..=3 {
        append(&mut tx, &m.id, seq, Event::Point(Team::A), "c", 10)
            .await
            .unwrap();
    }
    mark(&mut tx, &m.id, &[2, 3], EventState::Undone)
        .await
        .unwrap();
    mark(&mut tx, &m.id, &[3], EventState::Discarded)
        .await
        .unwrap();
    mark(&mut tx, &m.id, &[], EventState::Replaced)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let evs = s.events(&m.id).await.unwrap();
    let states: Vec<EventState> = evs.iter().map(|e| e.state).collect();
    assert_eq!(
        states,
        vec![
            EventState::Effective,
            EventState::Undone,
            EventState::Discarded
        ]
    );
    assert_eq!(s.head_seq(&m.id).await.unwrap(), 3);
}

#[tokio::test]
async fn touch_bumps_revision_and_activity() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    touch(&mut tx, &m.id, 50).await.unwrap();
    touch(&mut tx, &m.id, 60).await.unwrap();
    tx.commit().await.unwrap();
    let m = s.by_scorer_token(&m.scorer_token).await.unwrap().unwrap();
    assert_eq!(m.revision, 2);
    assert_eq!(m.last_activity_at, 60);
}

#[tokio::test]
async fn results_are_stored_per_client_id() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    assert!(s.find_result(&m.id, "c1").await.unwrap().is_none());
    let mut tx = s.begin().await.unwrap();
    save_result(&mut tx, &m.id, "c1", 200, r#"{"ok":true}"#)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let r = s.find_result(&m.id, "c1").await.unwrap().unwrap();
    assert_eq!((r.status, r.body.as_str()), (200, r#"{"ok":true}"#));
    assert!(s.find_result(&m.id, "c2").await.unwrap().is_none());
}

#[tokio::test]
async fn rolled_back_transaction_leaves_nothing() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    append(&mut tx, &m.id, 1, Event::TimeUp, "c", 1)
        .await
        .unwrap();
    drop(tx);
    assert!(s.events(&m.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn file_database_uses_wal_and_foreign_keys() {
    let path = std::env::temp_dir().join(format!("padel-store-{}.db", uuid::Uuid::new_v4()));
    let pool = db::connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .unwrap();
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((mode.as_str(), fk), ("wal", 1));
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[tokio::test]
async fn deleting_a_match_removes_events_and_results() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    let s = Store::new(pool.clone());
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    append(&mut tx, &m.id, 1, Event::TimeUp, "c", 1)
        .await
        .unwrap();
    save_result(&mut tx, &m.id, "c", 200, "{}").await.unwrap();
    tx.commit().await.unwrap();
    sqlx::query("DELETE FROM matches WHERE id = ?")
        .bind(&m.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(s.events(&m.id).await.unwrap().is_empty());
    assert!(s.find_result(&m.id, "c").await.unwrap().is_none());
}

#[tokio::test]
async fn mark_unknown_seq_errors() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    append(&mut tx, &m.id, 1, Event::TimeUp, "c", 1)
        .await
        .unwrap();
    let r = mark(&mut tx, &m.id, &[1, 9], EventState::Undone).await;
    assert!(matches!(r, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn touch_unknown_match_errors() {
    let s = store().await;
    let mut tx = s.begin().await.unwrap();
    assert!(matches!(
        touch(&mut tx, "missing", 1).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn duplicate_seq_append_is_refused() {
    let s = store().await;
    let m = s.create_match(new_match(), 1).await.unwrap();
    let mut tx = s.begin().await.unwrap();
    append(&mut tx, &m.id, 1, Event::TimeUp, "c", 1)
        .await
        .unwrap();
    let r = append(&mut tx, &m.id, 1, Event::TimeUp, "d", 1).await;
    assert!(matches!(r, Err(StoreError::Db(_))));
}
