#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Command, CommandOutcome, CommandRequest, MatchView, ReplaceRequest, apply_command, replace_from,
};
use padel_score_server::db;
use padel_score_server::results::{self, FinalScore};
use padel_score_server::store::{NewMatch, Store, StoredMatch, Teams};
use padel_scoring::{
    DecidingSet, DeuceRule, ErrorKind, Format, Player, SetRules, SetScore, SetsFormat, Start, Team,
};
use sqlx::{Row, SqlitePool};

const NOW: i64 = 1_000;
const OWNER: &str = "user-ana";

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

async fn new_match(s: &Store, format: Format, owner: Option<&str>) -> StoredMatch {
    s.create_match(
        NewMatch {
            format,
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
            owner_user_id: owner.map(str::to_owned),
            players: Default::default(),
        },
        NOW,
    )
    .await
    .unwrap()
}

fn points(team: Team, n: usize) -> Vec<Command> {
    vec![Command::Point { team }; n]
}

fn live(client_id: &str, commands: Vec<Command>) -> CommandRequest {
    CommandRequest {
        client_id: client_id.into(),
        batch: false,
        base_revision: 0,
        commands,
    }
}

fn applied(o: CommandOutcome) -> MatchView {
    match o {
        CommandOutcome::Applied(v) => v,
        other => panic!("expected Applied, got {other:?}"),
    }
}

/// One live request with 24 points for `team`: a 6-0 one-set match.
async fn finish(s: &Store, m: &StoredMatch, team: Team, now: i64) -> MatchView {
    applied(
        apply_command(s, m, live("finish", points(team, 24)), now)
            .await
            .unwrap(),
    )
}

struct ResultRow {
    owner: String,
    winner: Option<String>,
    score: FinalScore,
    finished_at: i64,
}

async fn result_rows(pool: &SqlitePool, match_id: &str) -> Vec<ResultRow> {
    sqlx::query(
        "SELECT owner_user_id, winner, score, finished_at FROM match_results WHERE match_id = ?",
    )
    .bind(match_id)
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|r| ResultRow {
        owner: r.get("owner_user_id"),
        winner: r.get("winner"),
        score: serde_json::from_str(r.get::<&str, _>("score")).unwrap(),
        finished_at: r.get("finished_at"),
    })
    .collect()
}

async fn count_rows(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM match_results")
        .fetch_one(pool)
        .await
        .unwrap()
}

fn six_love() -> Vec<SetScore> {
    vec![SetScore {
        a: 6,
        b: 0,
        tiebreak: None,
    }]
}

#[tokio::test]
async fn finished_owned_match_writes_result() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &m, Team::A, NOW + 5).await;
    let rows = result_rows(&pool, &m.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.owner, OWNER);
    assert_eq!(row.winner.as_deref(), Some("a"));
    assert_eq!(row.score.sets, six_love());
    assert_eq!(row.score.games, [6, 0]);
    assert!(!row.score.timed);
    assert!(!row.score.overtime);
    assert_eq!(row.finished_at, NOW + 5);
}

#[tokio::test]
async fn anonymous_match_writes_nothing() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), None).await;
    finish(&s, &m, Team::A, NOW + 5).await;
    assert_eq!(count_rows(&pool).await, 0);
}

#[tokio::test]
async fn in_progress_match_has_no_result() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    applied(
        apply_command(&s, &m, live("c1", points(Team::A, 23)), NOW + 1)
            .await
            .unwrap(),
    );
    assert_eq!(count_rows(&pool).await, 0);
}

#[tokio::test]
async fn undo_after_finish_retracts_result() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    let v = finish(&s, &m, Team::A, NOW + 5).await;
    assert_eq!(count_rows(&pool).await, 1);
    applied(
        apply_command(
            &s,
            &m,
            live(
                "undo",
                vec![Command::Undo {
                    target_seq: v.head_seq,
                }],
            ),
            NOW + 6,
        )
        .await
        .unwrap(),
    );
    assert_eq!(count_rows(&pool).await, 0);
}

#[tokio::test]
async fn refinish_counts_once() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    let v = finish(&s, &m, Team::A, NOW + 5).await;
    applied(
        apply_command(
            &s,
            &m,
            live(
                "undo",
                vec![Command::Undo {
                    target_seq: v.head_seq,
                }],
            ),
            NOW + 6,
        )
        .await
        .unwrap(),
    );
    applied(
        apply_command(&s, &m, live("again", points(Team::A, 1)), NOW + 7)
            .await
            .unwrap(),
    );
    let rows = result_rows(&pool, &m.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].finished_at, NOW + 7);
    assert_eq!(count_rows(&pool).await, 1);
}

#[tokio::test]
async fn batch_and_replace_sync_too() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    let v = applied(
        apply_command(
            &s,
            &m,
            CommandRequest {
                client_id: "batch".into(),
                batch: true,
                base_revision: 0,
                commands: points(Team::B, 24),
            },
            NOW + 3,
        )
        .await
        .unwrap(),
    );
    let rows = result_rows(&pool, &m.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].winner.as_deref(), Some("b"));
    assert_eq!(rows[0].finished_at, NOW + 3);

    // Keep mine: drop everything after seq 1 and play one point instead.
    applied(
        replace_from(
            &s,
            &m,
            ReplaceRequest {
                client_id: "replace".into(),
                base_revision: v.revision,
                base_seq: 1,
                commands: points(Team::A, 1),
            },
            NOW + 4,
        )
        .await
        .unwrap(),
    );
    assert_eq!(count_rows(&pool).await, 0);
}

#[tokio::test]
async fn timed_draw_has_null_winner() {
    let (s, pool) = setup().await;
    let m = new_match(
        &s,
        Format::Timed {
            deuce: DeuceRule::GoldenPoint,
            draws_allowed: true,
        },
        Some(OWNER),
    )
    .await;
    let mut commands = Vec::new();
    for team in [Team::A, Team::B, Team::A, Team::B] {
        commands.extend(points(team, 4));
    }
    commands.push(Command::TimeUp);
    applied(
        apply_command(&s, &m, live("timed", commands), NOW + 2)
            .await
            .unwrap(),
    );
    let rows = result_rows(&pool, &m.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].winner, None);
    assert!(rows[0].score.timed);
    assert!(!rows[0].score.overtime);
    assert_eq!(rows[0].score.regulation_games, [2, 2]);
}

#[tokio::test]
async fn rejected_command_leaves_result() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &m, Team::A, NOW + 5).await;
    let outcome = apply_command(&s, &m, live("late", points(Team::B, 1)), NOW + 9)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        CommandOutcome::Rejected {
            kind: ErrorKind::MatchOver,
            index: 0
        }
    );
    assert_eq!(outcome.to_http().unwrap().0, 422);
    let rows = result_rows(&pool, &m.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].winner.as_deref(), Some("a"));
    assert_eq!(rows[0].finished_at, NOW + 5);
}

#[tokio::test]
async fn backfill_fills_old_finished_matches() {
    let (s, pool) = setup().await;
    let done = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &done, Team::B, NOW + 5).await;
    // Not finished, and anonymous: neither gets a row.
    let open = new_match(&s, one_set(), Some(OWNER)).await;
    applied(
        apply_command(&s, &open, live("c1", points(Team::A, 3)), NOW + 6)
            .await
            .unwrap(),
    );
    let anon = new_match(&s, one_set(), None).await;
    finish(&s, &anon, Team::A, NOW + 7).await;

    sqlx::query("DELETE FROM match_results")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(results::backfill(&s).await.unwrap(), 1);
    let rows = result_rows(&pool, &done.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].winner.as_deref(), Some("b"));
    assert_eq!(rows[0].score.sets[0].b, 6);
    // finished_at is the match's last activity.
    assert_eq!(rows[0].finished_at, NOW + 5);
    assert_eq!(count_rows(&pool).await, 1);

    assert_eq!(results::backfill(&s).await.unwrap(), 0);
    assert_eq!(count_rows(&pool).await, 1);
}
