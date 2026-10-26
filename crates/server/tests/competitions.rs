#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, apply_command,
};
use padel_score_server::competitions::places::{Place, places, validate_places};
use padel_score_server::competitions::{
    Competition, Kind, MAX_TEAMS, NewCompetition, Status, TeamChange,
};
use padel_score_server::db;
use padel_score_server::players::PlayerChange;
use padel_score_server::store::{Store, StoreError};
use padel_scoring::{DeuceRule, Format, SetRules, Team};
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
    assert_eq!(first.settings, json!({ "full_placement": false }));
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
async fn tournament_settings_are_normalized() {
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
        json!({ "full_placement": false })
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

// ---- lifecycle, placings and history context ----

fn place(team: &str, place: u32, place_to: u32) -> Place {
    Place {
        team_id: team.to_owned(),
        place,
        place_to,
    }
}

fn ids(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("t{i}")).collect()
}

fn ranges(spec: &[(u32, u32)]) -> Vec<Place> {
    spec.iter()
        .enumerate()
        .map(|(i, (p, to))| place(&format!("t{}", i + 1), *p, *to))
        .collect()
}

#[test]
fn places_must_tile_the_field() {
    let teams = ids(8);
    let valid: [&[(u32, u32)]; 3] = [
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 8),
        ],
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 8),
            (5, 8),
            (5, 8),
            (5, 8),
        ],
        &[
            (1, 1),
            (2, 2),
            (3, 4),
            (3, 4),
            (5, 8),
            (5, 8),
            (5, 8),
            (5, 8),
        ],
    ];
    for spec in valid {
        validate_places(&teams, &ranges(spec)).unwrap();
    }
    let invalid: [&[(u32, u32)]; 6] = [
        // a gap
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 8),
            (8, 8),
        ],
        // an overlap
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (7, 7),
        ],
        // a tie with the wrong number of teams
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 8),
            (5, 8),
            (5, 8),
            (1, 1),
        ],
        // place_to below place
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 7),
        ],
        // place_to past the field
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 5),
            (6, 6),
            (7, 7),
            (8, 9),
        ],
        // three teams at 5..8
        &[
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 8),
            (5, 8),
            (5, 8),
            (4, 4),
        ],
    ];
    for spec in invalid {
        let err = validate_places(&teams, &ranges(spec)).unwrap_err();
        assert!(
            matches!(err, StoreError::InvalidField("places")),
            "{spec:?}"
        );
    }
    // a team missing, a team twice, an unknown team
    let mut full = ranges(invalid_free());
    full.pop();
    assert!(validate_places(&teams, &full).is_err());
    let mut twice = ranges(invalid_free());
    twice[7].team_id = "t1".into();
    assert!(validate_places(&teams, &twice).is_err());
    let mut unknown = ranges(invalid_free());
    unknown[7].team_id = "nobody".into();
    assert!(validate_places(&teams, &unknown).is_err());
}

fn invalid_free() -> &'static [(u32, u32)] {
    &[
        (1, 1),
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 5),
        (6, 6),
        (7, 7),
        (8, 8),
    ]
}

async fn event_of(s: &Store, owner: &str, pairs: &[[&str; 2]]) -> (Competition, Vec<String>) {
    let ev = s
        .create_competition(owner, new_event("Sunday cup"), 10)
        .await
        .unwrap();
    let mut teams = Vec::new();
    for [p1, p2] in pairs {
        let players = [player(s, owner, p1).await, player(s, owner, p2).await];
        teams.push(
            s.add_team(owner, &ev.id, players, None, 10)
                .await
                .unwrap()
                .id,
        );
    }
    (ev, teams)
}

const FOUR: [[&str; 2]; 4] = [
    ["Ana", "Marta"],
    ["Rui", "Joao"],
    ["Eva", "Ines"],
    ["Tiago", "Luis"],
];

/// A running event of four teams with one court. The status is set directly, not through
/// `start_competition`, so the tournament's draw makes no matches: these tests are about the
/// shared event code, with matches made by hand.
async fn running_event(s: &Store, pool: &SqlitePool) -> (Competition, Vec<String>) {
    let (mut ev, teams) = event_of(s, A, &FOUR).await;
    s.add_court(A, &ev.id, None).await.unwrap();
    sqlx::query("UPDATE competitions SET status = 'running', started_at = 20 WHERE id = ?")
        .bind(&ev.id)
        .execute(pool)
        .await
        .unwrap();
    ev.status = Status::Running;
    ev.started_at = Some(20);
    (ev, teams)
}

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: padel_scoring::SetsFormat::One,
        deciding: padel_scoring::DecidingSet::FullSet,
    })
}

async fn one_set_event(s: &Store, pool: &SqlitePool) -> (Competition, Vec<String>) {
    let (ev, teams) = running_event(s, pool).await;
    sqlx::query("UPDATE competitions SET format = ? WHERE id = ?")
        .bind(serde_json::to_string(&one_set()).unwrap())
        .bind(&ev.id)
        .execute(pool)
        .await
        .unwrap();
    (ev, teams)
}

#[tokio::test]
async fn start_needs_teams_and_courts() {
    let (s, _) = setup().await;
    let (ev, _) = event_of(&s, A, &[["Ana", "Marta"]]).await;
    let err = s.start_competition(A, &ev.id, 20).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_few_teams")),
        "{err:?}"
    );
    let (ev, _) = event_of(&s, A, &FOUR).await;
    let err = s.start_competition(A, &ev.id, 20).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict("no_courts")), "{err:?}");
    s.add_court(A, &ev.id, None).await.unwrap();
    let started = s.start_competition(A, &ev.id, 20).await.unwrap();
    assert_eq!(started.status, Status::Running);
    assert_eq!(started.started_at, Some(20));
    let err = s.start_competition(B, &ev.id, 20).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn start_only_from_setup() {
    let (s, pool) = setup().await;
    let (ev, _) = running_event(&s, &pool).await;
    let err = s.start_competition(A, &ev.id, 30).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_started")),
        "{err:?}"
    );
    let got = s.competition(A, &ev.id).await.unwrap().unwrap();
    assert_eq!(got.started_at, Some(20));
}

fn four_places(spec: [(u32, u32); 4], teams: &[String]) -> Vec<Place> {
    spec.iter()
        .zip(teams)
        .map(|((p, to), t)| place(t, *p, *to))
        .collect()
}

#[tokio::test]
async fn finish_writes_two_placings_per_team() {
    let (s, pool) = setup().await;
    let (ev, teams) = running_event(&s, &pool).await;
    let given = four_places([(1, 1), (2, 2), (3, 3), (4, 4)], &teams);
    let (done, rotated) = s
        .finish_competition(A, &ev.id, Some(given.clone()), 90)
        .await
        .unwrap();
    assert_eq!(done.status, Status::Finished);
    assert_eq!(done.finished_at, Some(90));
    assert!(rotated.is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM placings WHERE event_id = ?")
        .bind(&ev.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 8);
    let ana = s.list_players(A).await.unwrap();
    let ana = ana.iter().find(|p| p.name == "Ana").unwrap();
    let placings = s.placings(A, &ana.id).await.unwrap();
    assert_eq!(placings.len(), 1);
    assert_eq!(placings[0].place, 1);
    assert_eq!(placings[0].field_size, 4);
    assert_eq!(placings[0].event_name, "Sunday cup");
    assert_eq!(placings[0].event_kind, "tournament");
    assert_eq!(placings[0].finished_at, 90);
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(places(&mut conn, &ev.id).await.unwrap(), given);
}

#[tokio::test]
async fn finish_shared_place() {
    let (s, pool) = setup().await;
    let (ev, teams) = running_event(&s, &pool).await;
    let given = four_places([(1, 1), (2, 2), (3, 4), (3, 4)], &teams);
    s.finish_competition(A, &ev.id, Some(given), 90)
        .await
        .unwrap();
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT p.name, pl.place, pl.place_to FROM placings pl \
         JOIN players p ON p.id = pl.player_id WHERE pl.event_id = ? ORDER BY p.name",
    )
    .bind(&ev.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    for name in ["Eva", "Ines", "Tiago", "Luis"] {
        let row = rows.iter().find(|r| r.0 == name).unwrap();
        assert_eq!((row.1, row.2), (3, 4), "{name}");
    }
}

#[tokio::test]
async fn finish_without_places_is_422() {
    let (s, pool) = setup().await;
    let (ev, teams) = running_event(&s, &pool).await;
    let err = s.finish_competition(A, &ev.id, None, 90).await.unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("places")), "{err:?}");
    let bad = four_places([(1, 1), (2, 2), (3, 3), (3, 3)], &teams);
    let err = s
        .finish_competition(A, &ev.id, Some(bad), 90)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("places")), "{err:?}");
    let ev = s.competition(A, &ev.id).await.unwrap().unwrap();
    assert_eq!(ev.status, Status::Running);
    let err = s.finish_competition(B, &ev.id, None, 90).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

async fn scorer_token(pool: &SqlitePool, match_id: &str) -> String {
    sqlx::query_scalar("SELECT scorer_token FROM matches WHERE id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 24 points for team A: a 6-0 one-set match.
async fn win_for_a(s: &Store, match_id: &str) {
    let (m, _) = s.snapshot(match_id).await.unwrap().unwrap();
    let req = CommandRequest {
        client_id: "finish".into(),
        batch: false,
        base_revision: 0,
        commands: vec![Command::Point { team: Team::A }; 24],
    };
    match apply_command(s, &m, req, 50, Authority::Scorer)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(_) => {}
        other => panic!("expected Applied, got {other:?}"),
    }
}

#[tokio::test]
async fn finish_rotates_match_tokens_and_clears_courts() {
    let (s, pool) = setup().await;
    let (ev, teams) = one_set_event(&s, &pool).await;
    let court = padel_score_server::competitions::courts::courts_in(
        &mut pool.acquire().await.unwrap(),
        &ev.id,
    )
    .await
    .unwrap()
    .remove(0);
    let m1 = s
        .create_event_match(A, &ev.id, &teams[0], &teams[1], None, Some(&court.id), 30)
        .await
        .unwrap();
    let m2 = s
        .create_event_match(A, &ev.id, &teams[2], &teams[3], None, None, 30)
        .await
        .unwrap();
    let before = (
        scorer_token(&pool, &m1.match_id).await,
        scorer_token(&pool, &m2.match_id).await,
    );
    let given = four_places([(1, 1), (2, 2), (3, 3), (4, 4)], &teams);
    let (_, mut rotated) = s
        .finish_competition(A, &ev.id, Some(given), 90)
        .await
        .unwrap();
    rotated.sort();
    let mut expected = vec![m1.match_id.clone(), m2.match_id.clone()];
    expected.sort();
    assert_eq!(rotated, expected);
    assert_ne!(scorer_token(&pool, &m1.match_id).await, before.0);
    assert_ne!(scorer_token(&pool, &m2.match_id).await, before.1);
    let current: Option<String> =
        sqlx::query_scalar("SELECT current_match_id FROM courts WHERE id = ?")
            .bind(&court.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(current, None);
}

#[tokio::test]
async fn event_match_in_history_has_event_context() {
    let (s, pool) = setup().await;
    let (ev, teams) = one_set_event(&s, &pool).await;
    let m = s
        .create_event_match(A, &ev.id, &teams[0], &teams[1], None, None, 30)
        .await
        .unwrap();
    win_for_a(&s, &m.match_id).await;
    let quick = s
        .create_match(
            padel_score_server::store::NewMatch {
                format: one_set(),
                start: padel_scoring::Start {
                    first_server: padel_scoring::Player {
                        team: Team::A,
                        index: 0,
                    },
                    other_first_server: 0,
                },
                teams: padel_score_server::store::Teams {
                    a: ["Ana".into(), "Rui".into()],
                    b: ["Eva".into(), "Tiago".into()],
                },
                owner_user_id: Some(A.to_owned()),
                players: Default::default(),
            },
            40,
        )
        .await
        .unwrap();
    win_for_a(&s, &quick.id).await;
    let (items, _) = s.history(A, None, 30).await.unwrap();
    assert_eq!(items.len(), 2);
    let with = items.iter().find(|e| e.match_id == m.match_id).unwrap();
    let json = serde_json::to_value(with).unwrap();
    assert_eq!(
        json["event"],
        json!({"id": ev.id, "name": "Sunday cup", "kind": "tournament", "label": "Match 1"})
    );
    let without = items.iter().find(|e| e.match_id == quick.id).unwrap();
    assert!(
        serde_json::to_value(without)
            .unwrap()
            .get("event")
            .is_none()
    );
    // and on a player's page
    let ana = s.list_players(A).await.unwrap();
    let ana = ana.iter().find(|p| p.name == "Ana").unwrap();
    let (page, _) = s
        .player_history(A, &ana.id, None, 30)
        .await
        .unwrap()
        .unwrap();
    let json = serde_json::to_value(&page).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["event"]["name"], "Sunday cup");
}

#[tokio::test]
async fn placings_are_owner_filtered() {
    let (s, pool) = setup().await;
    let (ev, teams) = running_event(&s, &pool).await;
    let given = four_places([(1, 1), (2, 2), (3, 3), (4, 4)], &teams);
    s.finish_competition(A, &ev.id, Some(given), 90)
        .await
        .unwrap();
    let ana = s.list_players(A).await.unwrap();
    let ana = ana.iter().find(|p| p.name == "Ana").unwrap();
    assert_eq!(s.placings(A, &ana.id).await.unwrap().len(), 1);
    assert!(s.placings(B, &ana.id).await.unwrap().is_empty());
}
