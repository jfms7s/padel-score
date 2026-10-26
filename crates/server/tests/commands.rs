#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, FinalRequest, MAX_COMMANDS, MatchView,
    ReplaceRequest, apply_command, record_final, replace_from, set_start, view_of,
};
use padel_score_server::db;
use padel_score_server::store::{EventState, NewMatch, Store, StoredEvent, StoredMatch, Teams};
use padel_scoring::{
    Current, DecidingSet, DeuceRule, ErrorKind, Event, FinalScore, Format, Player, PointCall,
    SetRules, SetScore, SetsFormat, Start, Status, Team,
};
use sqlx::SqlitePool;

const NOW: i64 = 1_000;

async fn store() -> Store {
    Store::new(db::connect("sqlite::memory:").await.unwrap())
}

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

async fn new_match(s: &Store) -> StoredMatch {
    s.create_match(
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
                a: ["Ana".into(), "Bea".into()],
                b: ["Cris".into(), "Dani".into()],
            },
            owner_user_id: None,
            players: Default::default(),
        },
        NOW,
    )
    .await
    .unwrap()
}

fn point(team: Team) -> Command {
    Command::Point { team }
}

fn undo(target_seq: i64) -> Command {
    Command::Undo { target_seq }
}

fn live(client_id: &str, commands: Vec<Command>) -> CommandRequest {
    CommandRequest {
        client_id: client_id.into(),
        batch: false,
        base_revision: 0,
        commands,
    }
}

fn batch(client_id: &str, base_revision: i64, commands: Vec<Command>) -> CommandRequest {
    CommandRequest {
        client_id: client_id.into(),
        batch: true,
        base_revision,
        commands,
    }
}

fn applied(o: CommandOutcome) -> MatchView {
    match o {
        CommandOutcome::Applied(v) => v,
        other => panic!("expected Applied, got {other:?}"),
    }
}

fn conflict(o: CommandOutcome) -> MatchView {
    match o {
        CommandOutcome::Conflict(v) => v,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

async fn run(s: &Store, m: &StoredMatch, req: CommandRequest) -> CommandOutcome {
    apply_command(s, m, req, NOW + 1, Authority::Scorer)
        .await
        .unwrap()
}

async fn revision(s: &Store, m: &StoredMatch) -> i64 {
    s.by_scorer_token(&m.scorer_token)
        .await
        .unwrap()
        .unwrap()
        .revision
}

fn seqs(v: &MatchView) -> Vec<i64> {
    v.events.iter().map(|e| e.seq).collect()
}

fn game(v: &MatchView) -> (PointCall, PointCall) {
    match &v.score.current {
        Current::Game(g) => (g.a, g.b),
        other => panic!("expected a game, got {other:?}"),
    }
}

/// Play `points` straight points for team A in one live request (24 is a 6-0 one-set match).
async fn win_set(s: &Store, m: &StoredMatch, points: usize) {
    applied(run(s, m, live("setup", vec![point(Team::A); points])).await);
}

#[tokio::test]
async fn live_points_append_in_order() {
    let s = store().await;
    let m = new_match(&s).await;
    let mut last = None;
    for (i, c) in ["c1", "c2", "c3"].into_iter().enumerate() {
        let v = applied(run(&s, &m, live(c, vec![point(Team::A)])).await);
        assert_eq!(v.head_seq, i as i64 + 1);
        last = Some(v);
    }
    let v = last.unwrap();
    assert_eq!(seqs(&v), vec![1, 2, 3]);
    assert_eq!(game(&v), (PointCall::Forty, PointCall::Love));
    assert_eq!(v.revision, 3);
    assert_eq!(revision(&s, &m).await, 3);
}

#[tokio::test]
async fn resend_with_same_client_id_is_not_counted_twice() {
    let s = store().await;
    let m = new_match(&s).await;
    let first = applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    let again = run(&s, &m, live("c1", vec![point(Team::A)])).await;
    match again {
        CommandOutcome::Replayed { status, body } => {
            assert_eq!(status, 200);
            let view: MatchView = serde_json::from_str(&body).unwrap();
            assert_eq!(view, first);
        }
        other => panic!("expected Replayed, got {other:?}"),
    }
    assert_eq!(s.events(&m.id).await.unwrap().len(), 1);
    assert_eq!(revision(&s, &m).await, 1);
}

#[tokio::test]
async fn stale_batch_is_rejected_with_current_state() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    let v = conflict(run(&s, &m, batch("b1", 0, vec![point(Team::B), point(Team::B)])).await);
    assert_eq!(v.revision, 1);
    assert_eq!(seqs(&v), vec![1]);
    assert_eq!(revision(&s, &m).await, 1);
    assert_eq!(s.events(&m.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn batch_made_before_an_undo_is_stale() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    // The batch was built at revision 1; an undo then lands, leaving head_seq unchanged.
    let after_undo = applied(run(&s, &m, live("c2", vec![undo(1)])).await);
    assert_eq!(after_undo.head_seq, 1);
    assert_eq!(after_undo.revision, 2);
    let v = conflict(run(&s, &m, batch("b1", 1, vec![point(Team::B)])).await);
    assert_eq!(v.revision, 2);
    assert!(v.events.is_empty());
    assert_eq!(s.events(&m.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn fresh_batch_is_applied() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    let v = applied(
        run(
            &s,
            &m,
            batch(
                "b1",
                1,
                vec![point(Team::A), point(Team::A), point(Team::B)],
            ),
        )
        .await,
    );
    assert_eq!(seqs(&v), vec![1, 2, 3, 4]);
    assert_eq!(game(&v), (PointCall::Forty, PointCall::Fifteen));
    // One applied request bumps the revision once, whatever its length.
    assert_eq!(v.revision, 2);
    let events = s.events(&m.id).await.unwrap();
    assert!(events.iter().all(|e| e.client_id == "b1" || e.seq == 1));
}

#[tokio::test]
async fn undo_removes_latest_point() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    applied(run(&s, &m, live("c2", vec![point(Team::B)])).await);
    let v = applied(run(&s, &m, live("c3", vec![undo(2)])).await);
    assert_eq!(seqs(&v), vec![1]);
    assert_eq!(game(&v), (PointCall::Fifteen, PointCall::Love));
    assert_eq!(v.head_seq, 2);
    let events = s.events(&m.id).await.unwrap();
    assert_eq!(events.len(), 2, "undo marks rows, it never deletes them");
    assert_eq!(events[1].state, EventState::Undone);
}

#[tokio::test]
async fn undo_of_non_latest_is_conflict() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    applied(run(&s, &m, live("c2", vec![point(Team::B)])).await);
    let v = conflict(run(&s, &m, live("c3", vec![undo(1)])).await);
    assert_eq!(seqs(&v), vec![1, 2]);
    assert_eq!(revision(&s, &m).await, 2);
    let events = s.events(&m.id).await.unwrap();
    assert!(events.iter().all(|e| e.state == EventState::Effective));
    // Undo on an empty candidate list is a conflict too.
    let empty = new_match(&s).await;
    conflict(run(&s, &empty, live("c4", vec![undo(1)])).await);
}

#[tokio::test]
async fn point_after_match_over_is_rejected_and_not_stored() {
    let s = store().await;
    let m = new_match(&s).await;
    win_set(&s, &m, 24).await;
    let before = s.events(&m.id).await.unwrap().len();
    assert_eq!(before, 24);
    let o = run(&s, &m, live("late", vec![point(Team::B)])).await;
    assert_eq!(
        o,
        CommandOutcome::Rejected {
            kind: ErrorKind::MatchOver,
            index: 0
        }
    );
    assert_eq!(s.events(&m.id).await.unwrap().len(), before);
    assert_eq!(revision(&s, &m).await, 1);
}

#[tokio::test]
async fn multi_command_request_is_all_or_nothing() {
    let s = store().await;
    let m = new_match(&s).await;
    win_set(&s, &m, 23).await; // 5-0, 40-0: the next point wins the match
    let o = run(&s, &m, live("two", vec![point(Team::A), point(Team::A)])).await;
    assert_eq!(
        o,
        CommandOutcome::Rejected {
            kind: ErrorKind::MatchOver,
            index: 1
        }
    );
    assert_eq!(s.events(&m.id).await.unwrap().len(), 23);
    assert_eq!(revision(&s, &m).await, 1);

    // A conflicting undo later in the request drops the earlier point as well.
    conflict(run(&s, &m, live("three", vec![point(Team::B), undo(999)])).await);
    assert_eq!(s.events(&m.id).await.unwrap().len(), 23);
    assert_eq!(revision(&s, &m).await, 1);
}

#[tokio::test]
async fn replace_marks_server_tail_and_appends_mine() {
    let s = store().await;
    let m = new_match(&s).await;
    for c in ["c1", "c2", "c3"] {
        applied(run(&s, &m, live(c, vec![point(Team::A)])).await);
    }
    let v = applied(
        replace_from(
            &s,
            &m,
            ReplaceRequest {
                client_id: "r1".into(),
                base_revision: 3,
                base_seq: 1,
                commands: vec![point(Team::B), point(Team::B)],
            },
            NOW + 2,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(seqs(&v), vec![1, 4, 5], "seqs are never reused");
    assert_eq!(v.head_seq, 5);
    assert_eq!(v.revision, 4);
    assert_eq!(game(&v), (PointCall::Fifteen, PointCall::Thirty));
    let events = s.events(&m.id).await.unwrap();
    let states: Vec<_> = events.iter().map(|e| (e.seq, e.state)).collect();
    assert_eq!(
        states,
        vec![
            (1, EventState::Effective),
            (2, EventState::Replaced),
            (3, EventState::Replaced),
            (4, EventState::Effective),
            (5, EventState::Effective),
        ]
    );
    assert!(s.find_result(&m.id, "r1").await.unwrap().is_some());
}

#[tokio::test]
async fn view_ignores_undone_and_replaced_rows() {
    let s = store().await;
    let m = new_match(&s).await;
    let row = |seq, team, state| StoredEvent {
        seq,
        event: Event::Point(team),
        client_id: "x".into(),
        state,
    };
    let events = vec![
        row(1, Team::A, EventState::Effective),
        row(2, Team::B, EventState::Undone),
        row(3, Team::B, EventState::Replaced),
        row(4, Team::A, EventState::Effective),
        row(5, Team::B, EventState::Discarded),
    ];
    let v = view_of(&m, &events).unwrap();
    assert_eq!(seqs(&v), vec![1, 4]);
    assert_eq!(v.head_seq, 5);
    assert_eq!(game(&v), (PointCall::Thirty, PointCall::Love));
    assert_eq!(v.id, m.id);
    assert_eq!(v.revision, m.revision);

    let empty = view_of(&m, &[]).unwrap();
    assert_eq!(empty.head_seq, 0);
}

#[tokio::test]
async fn conflict_is_not_saved_so_replace_can_follow() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    let v = conflict(run(&s, &m, batch("b1", 0, vec![point(Team::B)])).await);
    assert!(s.find_result(&m.id, "b1").await.unwrap().is_none());
    // A resend is evaluated again, not replayed.
    conflict(run(&s, &m, batch("b1", 0, vec![point(Team::B)])).await);
    // "Keep mine": replace the server tail from the batch's base.
    let r = applied(
        replace_from(
            &s,
            &m,
            ReplaceRequest {
                client_id: "r1".into(),
                base_revision: v.revision,
                base_seq: 0,
                commands: vec![point(Team::B)],
            },
            NOW + 2,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(seqs(&r), vec![2]);
    assert_eq!(game(&r), (PointCall::Love, PointCall::Fifteen));
}

#[tokio::test]
async fn resend_of_rejected_returns_same_rejection() {
    let s = store().await;
    let m = new_match(&s).await;
    win_set(&s, &m, 24).await;
    let first = run(&s, &m, live("late", vec![point(Team::B)])).await;
    assert!(matches!(first, CommandOutcome::Rejected { .. }));
    let (first_status, first_body) = first.to_http().unwrap();
    assert_eq!(first_status, 422);
    let again = run(&s, &m, live("late", vec![point(Team::B)])).await;
    match &again {
        CommandOutcome::Replayed { status, body } => {
            assert_eq!(*status, 422);
            assert_eq!(body, &first_body);
            let json: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                json,
                serde_json::json!({ "error": "rejected", "kind": "match_over", "index": 0 })
            );
        }
        other => panic!("expected Replayed, got {other:?}"),
    }
    assert_eq!(again.to_http().unwrap(), (first_status, first_body));
    assert_eq!(s.events(&m.id).await.unwrap().len(), 24);
}

#[tokio::test]
async fn undo_within_same_request_is_allowed() {
    let s = store().await;
    let m = new_match(&s).await;
    let v = applied(run(&s, &m, live("c1", vec![point(Team::A), undo(1)])).await);
    assert!(v.events.is_empty());
    assert_eq!(v.head_seq, 1);
    assert_eq!(v.revision, 1);
    let events = s.events(&m.id).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].state, EventState::Undone);
}

#[tokio::test]
async fn replace_rejects_stale_revision() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    applied(run(&s, &m, live("c2", vec![point(Team::A)])).await);
    let req = |client_id: &str, base_revision, base_seq| ReplaceRequest {
        client_id: client_id.into(),
        base_revision,
        base_seq,
        commands: vec![point(Team::B)],
    };
    let v = conflict(
        replace_from(&s, &m, req("r1", 1, 0), NOW, Authority::Scorer)
            .await
            .unwrap(),
    );
    assert_eq!(v.revision, 2);
    // base_seq beyond head_seq, or negative, is a conflict too.
    conflict(
        replace_from(&s, &m, req("r2", 2, 3), NOW, Authority::Scorer)
            .await
            .unwrap(),
    );
    conflict(
        replace_from(&s, &m, req("r3", 2, -1), NOW, Authority::Scorer)
            .await
            .unwrap(),
    );
    let events = s.events(&m.id).await.unwrap();
    assert!(events.iter().all(|e| e.state == EventState::Effective));
    assert_eq!(events.len(), 2);
    assert_eq!(revision(&s, &m).await, 2);
    assert!(s.find_result(&m.id, "r1").await.unwrap().is_none());
}

#[tokio::test]
async fn empty_or_oversized_request_is_invalid() {
    let s = store().await;
    let m = new_match(&s).await;
    assert!(matches!(
        run(&s, &m, live("c1", vec![])).await,
        CommandOutcome::Invalid(_)
    ));
    assert!(matches!(
        run(&s, &m, live("c2", vec![point(Team::A); MAX_COMMANDS + 1])).await,
        CommandOutcome::Invalid(_)
    ));
    assert!(matches!(
        run(&s, &m, live("", vec![point(Team::A)])).await,
        CommandOutcome::Invalid(_)
    ));
    assert!(s.find_result(&m.id, "c1").await.unwrap().is_none());
    assert!(s.events(&m.id).await.unwrap().is_empty());
    assert_eq!(revision(&s, &m).await, 0);
}

#[tokio::test]
async fn request_over_1000_commands_is_invalid() {
    // A whole match scored offline goes as one batch (or one replace): 1000 fits, 1001 doesn't.
    assert_eq!(MAX_COMMANDS, 1000);
    let s = store().await;
    let m = new_match(&s).await;
    assert!(matches!(
        run(&s, &m, live("c1", vec![point(Team::A); 1001])).await,
        CommandOutcome::Invalid(_)
    ));
    let alternating: Vec<Command> = (0..1000)
        .map(|i| point(if i % 2 == 0 { Team::A } else { Team::B }))
        .collect();
    assert!(!matches!(
        run(&s, &m, live("c2", alternating)).await,
        CommandOutcome::Invalid(_)
    ));
}

#[test]
fn command_wire_shape() {
    let cmds: Vec<Command> = serde_json::from_str(
        r#"[{"kind":"point","team":"a"},{"kind":"time_up"},{"kind":"undo","target_seq":17}]"#,
    )
    .unwrap();
    assert_eq!(cmds, vec![point(Team::A), Command::TimeUp, undo(17)]);
    let req: CommandRequest = serde_json::from_str(
        r#"{"client_id":"u","batch":false,"base_revision":9,"commands":[{"kind":"point","team":"b"}]}"#,
    )
    .unwrap();
    assert_eq!(req.commands, vec![point(Team::B)]);
}

// Typed results, the coordinator lock and choosing the server (phase 6).

const OWNER: &str = "user-ana";

async fn store_and_pool() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

/// Like `new_match`, but owned: the results projection only follows owned matches.
async fn owned_match(s: &Store) -> StoredMatch {
    let m = new_match(s).await;
    s.create_match(
        NewMatch {
            format: m.format,
            start: m.start,
            teams: m.teams,
            owner_user_id: Some(OWNER.into()),
            players: Default::default(),
        },
        NOW,
    )
    .await
    .unwrap()
}

fn six_three() -> FinalScore {
    FinalScore::Sets(vec![SetScore {
        a: 6,
        b: 3,
        tiebreak: None,
    }])
}

fn final_req(client_id: &str, base_revision: Option<i64>, score: FinalScore) -> FinalRequest {
    FinalRequest {
        client_id: client_id.into(),
        base_revision,
        score,
    }
}

async fn typed(
    s: &Store,
    m: &StoredMatch,
    req: FinalRequest,
    authority: Authority,
) -> CommandOutcome {
    record_final(s, m, req, authority, NOW + 2).await.unwrap()
}

fn locked(o: CommandOutcome) -> MatchView {
    match o {
        CommandOutcome::Locked(v) => v,
        other => panic!("expected Locked, got {other:?}"),
    }
}

async fn winners(pool: &SqlitePool, match_id: &str) -> Vec<Option<String>> {
    sqlx::query_scalar("SELECT winner FROM match_results WHERE match_id = ?")
        .bind(match_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

fn states(events: &[StoredEvent]) -> Vec<EventState> {
    events.iter().map(|e| e.state).collect()
}

#[tokio::test]
async fn typed_final_replaces_log_and_records_result() {
    let (s, pool) = store_and_pool().await;
    let m = owned_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::B); 3])).await);
    let v = applied(
        typed(
            &s,
            &m,
            final_req("f1", Some(1), six_three()),
            Authority::Scorer,
        )
        .await,
    );
    assert_eq!(
        v.score.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(
        v.score.sets,
        vec![SetScore {
            a: 6,
            b: 3,
            tiebreak: None
        }]
    );
    assert!(!v.locked);
    assert_eq!(v.revision, 2);
    // 9 games of 4 points each, appended after the three tapped points.
    assert_eq!(v.events.len(), 36);
    assert_eq!(v.events[0].seq, 4);
    let events = s.events(&m.id).await.unwrap();
    assert_eq!(states(&events[..3]), vec![EventState::Replaced; 3]);
    assert!(
        events[3..]
            .iter()
            .all(|e| e.state == EventState::Effective && e.client_id == "f1")
    );
    assert_eq!(winners(&pool, &m.id).await, vec![Some("a".to_owned())]);
}

#[tokio::test]
async fn typed_final_is_idempotent() {
    let s = store().await;
    let m = new_match(&s).await;
    let first = applied(
        typed(
            &s,
            &m,
            final_req("f1", Some(0), six_three()),
            Authority::Scorer,
        )
        .await,
    );
    match typed(
        &s,
        &m,
        final_req("f1", Some(0), six_three()),
        Authority::Scorer,
    )
    .await
    {
        CommandOutcome::Replayed { status, body } => {
            assert_eq!(status, 200);
            assert_eq!(serde_json::from_str::<MatchView>(&body).unwrap(), first);
        }
        other => panic!("expected Replayed, got {other:?}"),
    }
    assert_eq!(s.events(&m.id).await.unwrap().len(), 36);
    assert_eq!(revision(&s, &m).await, 1);
}

#[tokio::test]
async fn typed_final_stale_base_revision_conflicts() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    let v = conflict(
        typed(
            &s,
            &m,
            final_req("f1", Some(0), six_three()),
            Authority::Scorer,
        )
        .await,
    );
    assert_eq!(v.revision, 1);
    assert_eq!(seqs(&v), vec![1]);
    assert_eq!(
        states(&s.events(&m.id).await.unwrap()),
        vec![EventState::Effective]
    );
    assert_eq!(revision(&s, &m).await, 1);
    assert!(s.find_result(&m.id, "f1").await.unwrap().is_none());
}

#[tokio::test]
async fn typed_impossible_score_is_an_invalid_score_error() {
    let s = store().await;
    let m = new_match(&s).await;
    let bad = FinalScore::Sets(vec![SetScore {
        a: 6,
        b: 5,
        tiebreak: None,
    }]);
    let err = record_final(
        &s,
        &m,
        final_req("f1", Some(0), bad),
        Authority::Scorer,
        NOW,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        padel_score_server::store::StoreError::InvalidScore(
            padel_scoring::InvalidScore::ImpossibleSet { index: 0 }
        )
    ));
    assert_eq!(revision(&s, &m).await, 0);
}

#[tokio::test]
async fn typed_score_too_long_is_invalid() {
    let s = store().await;
    let m = new_match(&s).await;
    let long = FinalScore::Sets(vec![SetScore {
        a: 7,
        b: 6,
        tiebreak: Some((600, 598)),
    }]);
    assert!(matches!(
        typed(&s, &m, final_req("f1", Some(0), long), Authority::Scorer).await,
        CommandOutcome::Invalid("too many commands in one request")
    ));
    assert!(s.events(&m.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn coordinator_final_locks() {
    let s = store().await;
    let m = new_match(&s).await;
    applied(run(&s, &m, live("c1", vec![point(Team::B)])).await);
    // The coordinator's result wins whatever the revision.
    let v = applied(typed(&s, &m, final_req("f1", None, six_three()), Authority::Owner).await);
    assert!(v.locked);
    let json = serde_json::to_value(&v).unwrap();
    assert_eq!(json["locked"], serde_json::json!(true));
    let (stored, _) = s.snapshot(&m.id).await.unwrap().unwrap();
    assert_eq!(stored.result_locked_at, Some(NOW + 2));
}

#[tokio::test]
async fn locked_match_refuses_scorer_commands() {
    let s = store().await;
    let m = new_match(&s).await;
    let v = applied(typed(&s, &m, final_req("f1", None, six_three()), Authority::Owner).await);
    let head = v.head_seq;

    let seen = locked(run(&s, &m, live("p1", vec![point(Team::B)])).await);
    assert_eq!(seen, v);
    locked(run(&s, &m, live("u1", vec![undo(head)])).await);
    locked(run(&s, &m, batch("b1", v.revision, vec![point(Team::B)])).await);
    locked(
        replace_from(
            &s,
            &m,
            ReplaceRequest {
                client_id: "r1".into(),
                base_revision: v.revision,
                base_seq: 0,
                commands: vec![point(Team::B)],
            },
            NOW + 3,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    locked(
        typed(
            &s,
            &m,
            final_req("f2", Some(v.revision), six_three()),
            Authority::Scorer,
        )
        .await,
    );
    let start = Start {
        first_server: Player {
            team: Team::B,
            index: 1,
        },
        other_first_server: 1,
    };
    assert!(matches!(
        set_start(&s, &m, start, Authority::Scorer, NOW + 3).await,
        Err(padel_score_server::store::StoreError::Conflict("locked"))
    ));
    for client_id in ["p1", "u1", "b1", "r1", "f2"] {
        assert!(s.find_result(&m.id, client_id).await.unwrap().is_none());
    }
    assert_eq!(revision(&s, &m).await, v.revision);
    assert_eq!(s.events(&m.id).await.unwrap().len(), 36);
}

#[tokio::test]
async fn owner_commands_ignore_lock() {
    let s = store().await;
    let m = new_match(&s).await;
    let v = applied(typed(&s, &m, final_req("f1", None, six_three()), Authority::Owner).await);
    let after = applied(
        apply_command(
            &s,
            &m,
            live("u1", vec![undo(v.head_seq)]),
            NOW + 3,
            Authority::Owner,
        )
        .await
        .unwrap(),
    );
    assert_eq!(after.score.status, Status::InProgress);
    // The lock stays until the coordinator types another result.
    assert!(after.locked);
}

#[tokio::test]
async fn set_start_changes_the_server_before_the_first_point() {
    let s = store().await;
    let m = new_match(&s).await;
    let start = Start {
        first_server: Player {
            team: Team::B,
            index: 1,
        },
        other_first_server: 1,
    };
    let v = set_start(&s, &m, start, Authority::Scorer, NOW + 1)
        .await
        .unwrap();
    assert_eq!(v.start, start);
    assert_eq!(v.score.server, start.first_server);
    assert_eq!(v.revision, 1);
    let bad = Start {
        first_server: Player {
            team: Team::A,
            index: 2,
        },
        other_first_server: 0,
    };
    assert!(matches!(
        set_start(&s, &m, bad, Authority::Scorer, NOW + 1).await,
        Err(padel_score_server::store::StoreError::InvalidRules(_))
    ));
    applied(run(&s, &m, live("c1", vec![point(Team::A)])).await);
    assert!(matches!(
        set_start(&s, &m, start, Authority::Owner, NOW + 2).await,
        Err(padel_score_server::store::StoreError::Conflict(
            "already_started"
        ))
    ));
    // An undone point still counts as started: the log has a row.
    applied(run(&s, &m, live("c2", vec![undo(1)])).await);
    assert!(matches!(
        set_start(&s, &m, start, Authority::Scorer, NOW + 2).await,
        Err(padel_score_server::store::StoreError::Conflict(
            "already_started"
        ))
    ));
}

#[tokio::test]
async fn undo_after_typed_final_unfinishes() {
    let (s, pool) = store_and_pool().await;
    let m = owned_match(&s).await;
    let v = applied(
        typed(
            &s,
            &m,
            final_req("f1", Some(0), six_three()),
            Authority::Scorer,
        )
        .await,
    );
    assert_eq!(winners(&pool, &m.id).await.len(), 1);
    let after = applied(run(&s, &m, live("u1", vec![undo(v.head_seq)])).await);
    assert_eq!(after.score.status, Status::InProgress);
    assert!(winners(&pool, &m.id).await.is_empty());
}

#[test]
fn final_request_wire_shape() {
    let req: FinalRequest = serde_json::from_str(
        r#"{"client_id":"f","base_revision":3,"score":{"sets":[{"a":6,"b":4,"tiebreak":null}]}}"#,
    )
    .unwrap();
    assert_eq!(req.base_revision, Some(3));
    assert_eq!(
        req.score,
        FinalScore::Sets(vec![SetScore {
            a: 6,
            b: 4,
            tiebreak: None
        }])
    );
}
