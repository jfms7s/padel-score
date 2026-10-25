#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use padel_score_server::competitions::{
    Competition, Kind, MAX_TEAMS, NewCompetition, Status, TeamChange,
};
use padel_score_server::db;
use padel_score_server::players::PlayerChange;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DeuceRule, Format, SetRules};
use serde_json::json;
use sqlx::SqlitePool;

const A: &str = "user-a";
const B: &str = "user-b";

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn sets() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::GoldenPoint,
        sets: padel_scoring::SetsFormat::BestOf3,
        deciding: padel_scoring::DecidingSet::SuperTieBreak,
    })
}

fn timed() -> Format {
    Format::Timed {
        deuce: DeuceRule::GoldenPoint,
        draws_allowed: false,
    }
}

fn new_event(name: &str) -> NewCompetition {
    NewCompetition {
        name: name.to_owned(),
        kind: Kind::Tournament,
        format: sets(),
        settings: None,
    }
}

async fn player(s: &Store, owner: &str, name: &str) -> String {
    s.create_or_get_player(owner, name, 1).await.unwrap().0.id
}

async fn event(s: &Store) -> Competition {
    s.create_competition(A, new_event("Cup"), 10).await.unwrap()
}

async fn start(pool: &SqlitePool, id: &str, status: &str) {
    sqlx::query("UPDATE competitions SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn create_and_list_newest_first() {
    let (s, _) = setup().await;
    let first = s.create_competition(A, new_event("One"), 10).await.unwrap();
    let second = s.create_competition(A, new_event("Two"), 20).await.unwrap();
    assert_eq!(first.status, Status::Setup);
    assert_eq!(first.settings, json!({}));
    assert!(!first.spectator_token.is_empty());
    assert_ne!(first.spectator_token, second.spectator_token);
    let listed = s.competitions(A).await.unwrap();
    let names: Vec<_> = listed.iter().map(|(c, _, _)| c.name.as_str()).collect();
    assert_eq!(names, ["Two", "One"]);
    assert_eq!((listed[0].1, listed[0].2), (0, 0));
    assert_eq!(listed[0].0, second);
}

#[tokio::test]
async fn kind_format_mismatch_is_refused() {
    let (s, _) = setup().await;
    let mut tournament = new_event("T");
    tournament.format = timed();
    let err = s.create_competition(A, tournament, 1).await.unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("format")), "{err:?}");
    let non_stop = NewCompetition {
        name: "N".into(),
        kind: Kind::NonStop,
        format: sets(),
        settings: None,
    };
    let err = s.create_competition(A, non_stop, 1).await.unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("format")), "{err:?}");
    let ev = event(&s).await;
    let change = padel_score_server::competitions::CompetitionChange {
        format: Some(timed()),
        ..Default::default()
    };
    let err = s.update_competition(A, &ev.id, change).await.unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("format")), "{err:?}");
}

#[tokio::test]
async fn settings_must_be_empty_in_phase_6() {
    let (s, _) = setup().await;
    let mut new = new_event("T");
    new.settings = Some(json!({"x": 1}));
    let err = s.create_competition(A, new, 1).await.unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidField("settings")),
        "{err:?}"
    );
    let mut ok = new_event("T");
    ok.settings = Some(json!({}));
    assert_eq!(
        s.create_competition(A, ok, 1).await.unwrap().settings,
        json!({})
    );
}

#[tokio::test]
async fn blank_or_long_event_name_is_refused() {
    let (s, _) = setup().await;
    for name in ["", "   ", &"x".repeat(41)] {
        let err = s
            .create_competition(A, new_event(name), 1)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidField("name")), "{err:?}");
    }
    let ok = s
        .create_competition(A, new_event(&format!("  {}  ", "x".repeat(40))), 1)
        .await
        .unwrap();
    assert_eq!(ok.name.chars().count(), 40);
}

#[tokio::test]
async fn owners_do_not_share_events() {
    let (s, _) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let marta = player(&s, A, "Marta").await;
    assert!(s.competition(B, &ev.id).await.unwrap().is_none());
    assert!(s.competitions(B).await.unwrap().is_empty());
    assert!(s.teams(B, &ev.id).await.unwrap().is_none());
    let err = s
        .update_competition(B, &ev.id, Default::default())
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let err = s.delete_competition(B, &ev.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let err = s
        .add_team(B, &ev.id, [ana, marta], None, 1)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    assert!(s.competition(A, &ev.id).await.unwrap().is_some());
}

#[tokio::test]
async fn team_name_defaults_to_players() {
    let (s, _) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let marta = player(&s, A, "Marta").await;
    let team = s
        .add_team(A, &ev.id, [ana.clone(), marta.clone()], None, 1)
        .await
        .unwrap();
    assert_eq!(team.name, "Ana / Marta");
    assert_eq!(team.custom_name, None);
    s.update_player(
        A,
        &ana,
        PlayerChange {
            name: Some("Anabela".into()),
            archived: None,
        },
    )
    .await
    .unwrap();
    let teams = s.teams(A, &ev.id).await.unwrap().unwrap();
    assert_eq!(teams[0].name, "Anabela / Marta");
    assert_eq!(teams[0].players[0].name, "Anabela");
    let renamed = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: Some(Some("  Aces ".into())),
                players: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "Aces");
    assert_eq!(renamed.custom_name.as_deref(), Some("Aces"));
    let cleared = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: Some(None),
                players: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.name, "Anabela / Marta");
    let err = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: Some(Some(" ".into())),
                players: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("name")), "{err:?}");
    let custom = s
        .add_team(
            A,
            &ev.id,
            [player(&s, A, "Rui").await, player(&s, A, "Joana").await],
            Some("Dream".into()),
            2,
        )
        .await
        .unwrap();
    assert_eq!(custom.name, "Dream");
    assert_eq!(custom.position, 1);
}

#[tokio::test]
async fn team_with_unknown_archived_or_foreign_player_is_refused() {
    let (s, _) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let old = player(&s, A, "Old").await;
    s.update_player(
        A,
        &old,
        PlayerChange {
            name: None,
            archived: Some(true),
        },
    )
    .await
    .unwrap();
    let foreign = player(&s, B, "Eve").await;
    for other in ["no-such-player".to_owned(), old, foreign] {
        let err = s
            .add_team(A, &ev.id, [ana.clone(), other], None, 1)
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("players")),
            "{err:?}"
        );
    }
    let err = s
        .add_team(A, &ev.id, [ana.clone(), ana.clone()], None, 1)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidField("players")),
        "{err:?}"
    );
}

#[tokio::test]
async fn player_in_two_teams_is_refused() {
    let (s, _) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let marta = player(&s, A, "Marta").await;
    let rui = player(&s, A, "Rui").await;
    let joana = player(&s, A, "Joana").await;
    s.add_team(A, &ev.id, [ana.clone(), marta], None, 1)
        .await
        .unwrap();
    let err = s
        .add_team(A, &ev.id, [ana.clone(), rui.clone()], None, 1)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("player_in_team")),
        "{err:?}"
    );
    let team = s.add_team(A, &ev.id, [rui, joana], None, 1).await.unwrap();
    let err = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: None,
                players: Some([ana, team.players[1].id.clone()]),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("player_in_team")),
        "{err:?}"
    );
    // The same player in another event is fine.
    let other = s.create_competition(A, new_event("Two"), 20).await.unwrap();
    let ana = &s.teams(A, &ev.id).await.unwrap().unwrap()[0].players[0].id;
    let marta = &s.teams(A, &ev.id).await.unwrap().unwrap()[0].players[1].id;
    s.add_team(A, &other.id, [ana.clone(), marta.clone()], None, 1)
        .await
        .unwrap();
}

#[tokio::test]
async fn teams_frozen_after_start() {
    let (s, pool) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let marta = player(&s, A, "Marta").await;
    let rui = player(&s, A, "Rui").await;
    let team = s
        .add_team(A, &ev.id, [ana, marta.clone()], None, 1)
        .await
        .unwrap();
    start(&pool, &ev.id, "running").await;
    let err = s
        .add_team(
            A,
            &ev.id,
            [rui.clone(), player(&s, A, "Joana").await],
            None,
            2,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
    let err = s.delete_team(A, &ev.id, &team.id).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
    let err = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: None,
                players: Some([rui, marta]),
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
    let renamed = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: Some(Some("Aces".into())),
                players: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "Aces");
    start(&pool, &ev.id, "finished").await;
    let err = s
        .update_team(
            A,
            &ev.id,
            &team.id,
            TeamChange {
                name: Some(Some("Late".into())),
                players: None,
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_finished")),
        "{err:?}"
    );
}

#[tokio::test]
async fn team_cap() {
    let (s, _) = setup().await;
    let ev = event(&s).await;
    for i in 0..MAX_TEAMS {
        let p1 = player(&s, A, &format!("P{i}a")).await;
        let p2 = player(&s, A, &format!("P{i}b")).await;
        s.add_team(A, &ev.id, [p1, p2], None, 1).await.unwrap();
    }
    let p1 = player(&s, A, "Extra a").await;
    let p2 = player(&s, A, "Extra b").await;
    let err = s.add_team(A, &ev.id, [p1, p2], None, 1).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_many_teams")),
        "{err:?}"
    );
    assert_eq!(s.teams(A, &ev.id).await.unwrap().unwrap().len(), MAX_TEAMS);
}

#[tokio::test]
async fn delete_only_in_setup() {
    let (s, pool) = setup().await;
    let ev = event(&s).await;
    let ana = player(&s, A, "Ana").await;
    let marta = player(&s, A, "Marta").await;
    let team = s.add_team(A, &ev.id, [ana, marta], None, 1).await.unwrap();
    s.delete_team(A, &ev.id, &team.id).await.unwrap();
    let err = s.delete_team(A, &ev.id, &team.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let other = s.create_competition(A, new_event("Two"), 20).await.unwrap();
    start(&pool, &other.id, "running").await;
    let err = s.delete_competition(A, &other.id).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
    s.delete_competition(A, &ev.id).await.unwrap();
    assert!(s.competition(A, &ev.id).await.unwrap().is_none());
}

#[tokio::test]
async fn update_changes_name_and_format_only_in_setup() {
    let (s, pool) = setup().await;
    let ev = event(&s).await;
    let mut format = sets();
    if let Format::Sets(rules) = &mut format {
        rules.deuce = DeuceRule::Advantage;
    }
    let change = padel_score_server::competitions::CompetitionChange {
        name: Some(" Renamed ".into()),
        format: Some(format),
        settings: Some(json!({})),
    };
    let updated = s.update_competition(A, &ev.id, change).await.unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.format, format);
    start(&pool, &ev.id, "running").await;
    let change = padel_score_server::competitions::CompetitionChange {
        format: Some(sets()),
        ..Default::default()
    };
    let err = s.update_competition(A, &ev.id, change).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
}

#[tokio::test]
async fn existing_matches_load_with_lock_column() {
    let (s, pool) = setup().await;
    let format = serde_json::to_string(&sets()).unwrap();
    let start = serde_json::to_string(&padel_scoring::Start {
        first_server: padel_scoring::Player {
            team: padel_scoring::Team::A,
            index: 0,
        },
        other_first_server: 0,
    })
    .unwrap();
    let teams = json!({"a": ["Ana", "Rui"], "b": ["Marta", "Joana"]}).to_string();
    sqlx::query(
        "INSERT INTO matches (id, scorer_token, spectator_token, format, start, teams, \
         revision, owner_user_id, created_at, last_activity_at) \
         VALUES ('old', 'st', 'pt', ?, ?, ?, 0, NULL, 1, 1)",
    )
    .bind(format)
    .bind(start)
    .bind(teams)
    .execute(&pool)
    .await
    .unwrap();
    let m = s.by_scorer_token("st").await.unwrap().unwrap();
    assert_eq!(m.result_locked_at, None);
}
