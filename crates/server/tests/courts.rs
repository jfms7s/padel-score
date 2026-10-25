#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, apply_command,
};
use padel_score_server::competitions::courts::{EventMatchRow, MAX_COURTS, courts_in};
use padel_score_server::competitions::{Competition, Kind, NewCompetition};
use padel_score_server::db;
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use sqlx::{Row, SqlitePool};

const A: &str = "user-a";
const B: &str = "user-b";
const NOW: i64 = 1_000;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn player(s: &Store, name: &str) -> String {
    s.create_or_get_player(A, name, 1).await.unwrap().0.id
}

async fn set_status(pool: &SqlitePool, id: &str, status: &str) {
    sqlx::query("UPDATE competitions SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

/// A tournament of A with the given teams (pairs of player names), still in `setup`.
async fn event_with(s: &Store, pairs: &[[&str; 2]]) -> (Competition, Vec<String>) {
    let ev = s
        .create_competition(
            A,
            NewCompetition {
                name: "Cup".into(),
                kind: Kind::Tournament,
                format: one_set(),
                settings: None,
            },
            10,
        )
        .await
        .unwrap();
    let mut teams = Vec::new();
    for [p1, p2] in pairs {
        let players = [player(s, p1).await, player(s, p2).await];
        teams.push(s.add_team(A, &ev.id, players, None, 10).await.unwrap().id);
    }
    (ev, teams)
}

/// A running event with four teams.
async fn running(s: &Store, pool: &SqlitePool) -> (Competition, Vec<String>) {
    let (ev, teams) = event_with(
        s,
        &[
            ["Ana", "Marta"],
            ["Rui", "João"],
            ["Eva", "Inês"],
            ["Tiago", "Luís"],
        ],
    )
    .await;
    set_status(pool, &ev.id, "running").await;
    (ev, teams)
}

async fn new_match(
    s: &Store,
    ev: &Competition,
    teams: &[String],
    a: usize,
    b: usize,
    court: Option<&str>,
) -> EventMatchRow {
    s.create_event_match(A, &ev.id, &teams[a], &teams[b], None, court, NOW)
        .await
        .unwrap()
}

async fn scorer_token(pool: &SqlitePool, match_id: &str) -> String {
    sqlx::query_scalar("SELECT scorer_token FROM matches WHERE id = ?")
        .bind(match_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn current_of(pool: &SqlitePool, court_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT current_match_id FROM courts WHERE id = ?")
        .bind(court_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 24 points for team A: a 6-0 one-set match, which writes its `match_results` row.
async fn finish(s: &Store, match_id: &str) {
    let (m, _) = s.snapshot(match_id).await.unwrap().unwrap();
    let req = CommandRequest {
        client_id: "finish".into(),
        batch: false,
        base_revision: 0,
        commands: vec![Command::Point { team: Team::A }; 24],
    };
    match apply_command(s, &m, req, NOW, Authority::Scorer)
        .await
        .unwrap()
    {
        CommandOutcome::Applied(_) => {}
        other => panic!("expected Applied, got {other:?}"),
    }
}

#[tokio::test]
async fn courts_get_default_names_and_unique_tokens() {
    let (s, pool) = setup().await;
    let (ev, _) = running(&s, &pool).await;
    let one = s.add_court(A, &ev.id, None).await.unwrap();
    let two = s.add_court(A, &ev.id, None).await.unwrap();
    let named = s
        .add_court(A, &ev.id, Some("  Centre  ".into()))
        .await
        .unwrap();
    assert_eq!(one.name, "Court 1");
    assert_eq!(two.name, "Court 2");
    assert_eq!(named.name, "Centre");
    assert_eq!((one.position, two.position, named.position), (1, 2, 3));
    let tokens = [
        &one.scorer_token,
        &one.spectator_token,
        &two.scorer_token,
        &two.spectator_token,
    ];
    for (i, t) in tokens.iter().enumerate() {
        assert!(!t.is_empty());
        assert!(tokens.iter().skip(i + 1).all(|o| o != t));
    }
    assert_eq!(one.current_match_id, None);
    let renamed = s
        .rename_court(A, &ev.id, &one.id, "Pista 1".into())
        .await
        .unwrap();
    assert_eq!(renamed.name, "Pista 1");
    let err = s.add_court(A, &ev.id, Some(" ".into())).await.unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("name")), "{err:?}");
    // Another owner's event is unknown.
    let err = s.add_court(B, &ev.id, None).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    // A finished event takes no new court.
    set_status(&pool, &ev.id, "finished").await;
    let err = s.add_court(A, &ev.id, None).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("event_finished")),
        "{err:?}"
    );
    let mut conn = pool.acquire().await.unwrap();
    let listed = courts_in(&mut conn, &ev.id).await.unwrap();
    let names: Vec<_> = listed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Pista 1", "Court 2", "Centre"]);
}

#[tokio::test]
async fn event_match_records_players_and_names() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let row = new_match(&s, &ev, &teams, 0, 1, None).await;
    assert_eq!(row.label, "Match 1");
    assert_eq!(row.ordinal, 1);
    assert_eq!(row.event_id, ev.id);
    assert_eq!(
        (row.team_a_id.as_str(), row.team_b_id.as_str()),
        (teams[0].as_str(), teams[1].as_str())
    );
    assert_eq!((row.court_id.clone(), row.court_seq), (None, None));
    let (m, _) = s.snapshot(&row.match_id).await.unwrap().unwrap();
    assert_eq!(
        m.teams,
        Teams {
            a: ["Ana".into(), "Marta".into()],
            b: ["Rui".into(), "João".into()],
        }
    );
    assert_eq!(m.owner_user_id.as_deref(), Some(A));
    assert_eq!(m.format, ev.format);
    assert_eq!(
        m.start,
        Start {
            first_server: Player {
                team: Team::A,
                index: 0
            },
            other_first_server: 0,
        }
    );
    let rows = sqlx::query(
        "SELECT mp.team, mp.position, p.name FROM match_players mp \
         JOIN players p ON p.id = mp.player_id WHERE mp.match_id = ? ORDER BY mp.team, mp.position",
    )
    .bind(&row.match_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    let got: Vec<(String, i64, String)> = rows
        .iter()
        .map(|r| (r.get("team"), r.get("position"), r.get("name")))
        .collect();
    assert_eq!(
        got,
        [
            ("a".to_owned(), 0, "Ana".to_owned()),
            ("a".to_owned(), 1, "Marta".to_owned()),
            ("b".to_owned(), 0, "Rui".to_owned()),
            ("b".to_owned(), 1, "João".to_owned()),
        ]
    );
    let labelled = s
        .create_event_match(
            A,
            &ev.id,
            &teams[2],
            &teams[3],
            Some(" Final ".into()),
            None,
            NOW,
        )
        .await
        .unwrap();
    assert_eq!((labelled.label.as_str(), labelled.ordinal), ("Final", 2));
    let mut conn = pool.acquire().await.unwrap();
    let all = padel_score_server::competitions::courts::event_matches_in(&mut conn, &ev.id)
        .await
        .unwrap();
    assert_eq!(all, vec![row.clone(), labelled]);
    let (found_ev, found_row) =
        padel_score_server::competitions::courts::event_of_match_in(&mut conn, &row.match_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(found_ev.id, ev.id);
    assert_eq!(found_row, row);
}

#[tokio::test]
async fn event_match_allows_archived_players() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let ana = player(&s, "Ana").await;
    s.update_player(
        A,
        &ana,
        padel_score_server::players::PlayerChange {
            name: None,
            archived: Some(true),
        },
    )
    .await
    .unwrap();
    new_match(&s, &ev, &teams, 0, 1, None).await;
}

#[tokio::test]
async fn event_match_needs_running_event() {
    let (s, pool) = setup().await;
    let (ev, teams) = event_with(&s, &[["Ana", "Marta"], ["Rui", "João"]]).await;
    for status in ["setup", "finished"] {
        set_status(&pool, &ev.id, status).await;
        let err = s
            .create_event_match(A, &ev.id, &teams[0], &teams[1], None, None, NOW)
            .await
            .unwrap_err();
        assert!(
            matches!(err, StoreError::Conflict("event_not_running")),
            "{status}: {err:?}"
        );
    }
    set_status(&pool, &ev.id, "running").await;
    let err = s
        .create_event_match(B, &ev.id, &teams[0], &teams[1], None, None, NOW)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn event_match_teams_must_be_distinct_and_in_event() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let (other, other_teams) = event_with(&s, &[["Zé", "Rita"], ["Bia", "Leo"]]).await;
    set_status(&pool, &other.id, "running").await;
    let cases = [
        (teams[0].clone(), teams[0].clone()),
        (teams[0].clone(), other_teams[0].clone()),
        ("nope".to_owned(), teams[1].clone()),
    ];
    for (a, b) in cases {
        let err = s
            .create_event_match(A, &ev.id, &a, &b, None, None, NOW)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidField("teams")), "{err:?}");
    }
    let err = s
        .create_event_match(A, &ev.id, &teams[0], &teams[1], Some("".into()), None, NOW)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::InvalidField("label")), "{err:?}");
    let err = s
        .create_event_match(A, &ev.id, &teams[0], &teams[1], None, Some("nope"), NOW)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidField("court_id")),
        "{err:?}"
    );
    // Nothing was left behind by the refused creates.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM matches")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn assign_to_idle_court_becomes_current() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let row = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    assert_eq!(row.court_id.as_deref(), Some(court.id.as_str()));
    assert_eq!(row.court_seq, Some(1));
    assert_eq!(
        current_of(&pool, &court.id).await,
        Some(row.match_id.clone())
    );
    // Assigned after creation.
    let court2 = s.add_court(A, &ev.id, None).await.unwrap();
    let later = new_match(&s, &ev, &teams, 2, 3, None).await;
    let rotated = s
        .assign_match(A, &ev.id, &later.match_id, Some(&court2.id))
        .await
        .unwrap();
    assert_eq!(rotated, None);
    assert_eq!(current_of(&pool, &court2.id).await, Some(later.match_id));
}

#[tokio::test]
async fn assign_to_busy_court_queues() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let first = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    let second = new_match(&s, &ev, &teams, 2, 3, Some(&court.id)).await;
    let third = new_match(&s, &ev, &teams, 0, 2, None).await;
    s.assign_match(A, &ev.id, &third.match_id, Some(&court.id))
        .await
        .unwrap();
    assert_eq!(current_of(&pool, &court.id).await, Some(first.match_id));
    assert_eq!(second.court_seq, Some(2));
    let mut conn = pool.acquire().await.unwrap();
    let all = padel_score_server::competitions::courts::event_matches_in(&mut conn, &ev.id)
        .await
        .unwrap();
    assert_eq!(all[2].court_seq, Some(3));
    drop(conn);
    // An unknown court in the body, and another owner, are refused.
    let err = s
        .assign_match(A, &ev.id, &third.match_id, Some("nope"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::InvalidField("court_id")),
        "{err:?}"
    );
    let err = s
        .assign_match(B, &ev.id, &third.match_id, None)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let err = s.assign_match(A, &ev.id, "nope", None).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn advance_refuses_unfinished_current() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let first = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    new_match(&s, &ev, &teams, 2, 3, Some(&court.id)).await;
    let err = s.advance_court(A, &ev.id, &court.id).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("match_not_finished")),
        "{err:?}"
    );
    assert_eq!(current_of(&pool, &court.id).await, Some(first.match_id));
    // An empty court advances to nothing.
    let idle = s.add_court(A, &ev.id, None).await.unwrap();
    let done = s.advance_court(A, &ev.id, &idle.id).await.unwrap();
    assert_eq!((done.current, done.rotated), (None, None));
    let err = s.advance_court(B, &ev.id, &court.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn advance_rotates_old_scorer_token() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let first = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    let second = new_match(&s, &ev, &teams, 2, 3, Some(&court.id)).await;
    let old = scorer_token(&pool, &first.match_id).await;
    let spectator: String = sqlx::query_scalar("SELECT spectator_token FROM matches WHERE id = ?")
        .bind(&first.match_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    finish(&s, &first.match_id).await;
    let done = s.advance_court(A, &ev.id, &court.id).await.unwrap();
    assert_eq!(done.current.as_deref(), Some(second.match_id.as_str()));
    assert_eq!(done.rotated.as_deref(), Some(first.match_id.as_str()));
    assert!(s.by_scorer_token(&old).await.unwrap().is_none());
    assert!(s.by_spectator_token(&spectator).await.unwrap().is_some());
    assert_eq!(
        current_of(&pool, &court.id).await,
        Some(second.match_id.clone())
    );
    // A finished match cannot be reassigned.
    let err = s
        .assign_match(A, &ev.id, &first.match_id, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("match_finished")),
        "{err:?}"
    );
    // The court phone advances by its court token.
    finish(&s, &second.match_id).await;
    let done = s.advance_court_by_token(&court.scorer_token).await.unwrap();
    assert_eq!(done.current, None);
    assert_eq!(done.rotated.as_deref(), Some(second.match_id.as_str()));
    assert_eq!(current_of(&pool, &court.id).await, None);
    let err = s.advance_court_by_token("nope").await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn moving_current_match_rotates_and_promotes_next() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let one = s.add_court(A, &ev.id, None).await.unwrap();
    let two = s.add_court(A, &ev.id, None).await.unwrap();
    let first = new_match(&s, &ev, &teams, 0, 1, Some(&one.id)).await;
    let second = new_match(&s, &ev, &teams, 2, 3, Some(&one.id)).await;
    let busy = new_match(&s, &ev, &teams, 0, 2, Some(&two.id)).await;
    let old = scorer_token(&pool, &first.match_id).await;
    let rotated = s
        .assign_match(A, &ev.id, &first.match_id, Some(&two.id))
        .await
        .unwrap();
    assert_eq!(rotated.as_deref(), Some(first.match_id.as_str()));
    assert_ne!(scorer_token(&pool, &first.match_id).await, old);
    assert_eq!(
        current_of(&pool, &one.id).await,
        Some(second.match_id.clone())
    );
    assert_eq!(
        current_of(&pool, &two.id).await,
        Some(busy.match_id.clone())
    );
    // Queued behind `busy` on court two.
    let mut conn = pool.acquire().await.unwrap();
    let (_, row) =
        padel_score_server::competitions::courts::event_of_match_in(&mut conn, &first.match_id)
            .await
            .unwrap()
            .unwrap();
    drop(conn);
    assert_eq!(row.court_id.as_deref(), Some(two.id.as_str()));
    assert_eq!(row.court_seq, Some(2));
    // A queued match moved off rotates nothing.
    let rotated = s
        .assign_match(A, &ev.id, &first.match_id, None)
        .await
        .unwrap();
    assert_eq!(rotated, None);
    // Taking the current match off any court empties the court.
    let old = scorer_token(&pool, &second.match_id).await;
    let rotated = s
        .assign_match(A, &ev.id, &second.match_id, None)
        .await
        .unwrap();
    assert_eq!(rotated.as_deref(), Some(second.match_id.as_str()));
    assert_ne!(scorer_token(&pool, &second.match_id).await, old);
    assert_eq!(current_of(&pool, &one.id).await, None);
}

#[tokio::test]
async fn rotate_court_rotates_court_and_current_match_tokens() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let m = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    let old_match = scorer_token(&pool, &m.match_id).await;
    let rotated = s.rotate_court(A, &ev.id, &court.id).await.unwrap();
    assert_ne!(rotated.scorer_token, court.scorer_token);
    assert_eq!(rotated.spectator_token, court.spectator_token);
    assert_eq!(
        rotated.current_match_id.as_deref(),
        Some(m.match_id.as_str())
    );
    assert_ne!(scorer_token(&pool, &m.match_id).await, old_match);
    let err = s.rotate_court(B, &ev.id, &court.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn court_cap() {
    let (s, pool) = setup().await;
    let (ev, _) = running(&s, &pool).await;
    for _ in 0..MAX_COURTS {
        s.add_court(A, &ev.id, None).await.unwrap();
    }
    let err = s.add_court(A, &ev.id, None).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("too_many_courts")),
        "{err:?}"
    );
}

#[tokio::test]
async fn delete_court_in_use_is_conflict() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let used = s.add_court(A, &ev.id, None).await.unwrap();
    let free = s.add_court(A, &ev.id, None).await.unwrap();
    let m = new_match(&s, &ev, &teams, 0, 1, Some(&used.id)).await;
    finish(&s, &m.match_id).await;
    s.advance_court(A, &ev.id, &used.id).await.unwrap();
    // No current match now, but a match was played there.
    let err = s.delete_court(A, &ev.id, &used.id).await.unwrap_err();
    assert!(
        matches!(err, StoreError::Conflict("court_in_use")),
        "{err:?}"
    );
    let err = s.delete_court(B, &ev.id, &free.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    s.delete_court(A, &ev.id, &free.id).await.unwrap();
    let err = s.delete_court(A, &ev.id, &free.id).await.unwrap_err();
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
}

#[tokio::test]
async fn quick_match_create_unchanged() {
    let (s, _) = setup().await;
    let m = s
        .create_match(
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
                    a: [" Ana ".into(), "".into()],
                    b: ["Rui".into(), "João".into()],
                },
                owner_user_id: None,
                players: Default::default(),
            },
            NOW,
        )
        .await
        .unwrap();
    assert_eq!(m.teams.a, ["Ana".to_owned(), "Player 2".to_owned()]);
    assert!(s.by_scorer_token(&m.scorer_token).await.unwrap().is_some());
}

#[tokio::test]
async fn live_list_hides_queued_event_matches() {
    let (s, pool) = setup().await;
    let (ev, teams) = running(&s, &pool).await;
    let court = s.add_court(A, &ev.id, None).await.unwrap();
    let on_court = new_match(&s, &ev, &teams, 0, 1, Some(&court.id)).await;
    new_match(&s, &ev, &teams, 2, 3, Some(&court.id)).await;
    new_match(&s, &ev, &teams, 0, 2, None).await;
    let quick = s
        .create_match(
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
                    a: ["Ana".into(), "Rui".into()],
                    b: ["Marta".into(), "João".into()],
                },
                owner_user_id: Some(A.into()),
                players: Default::default(),
            },
            NOW,
        )
        .await
        .unwrap();
    let live = s.live_matches_for_owner(A).await.unwrap();
    let mut ids: Vec<_> = live.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    let mut want = vec![on_court.match_id, quick.id];
    want.sort();
    assert_eq!(ids, want);
}
