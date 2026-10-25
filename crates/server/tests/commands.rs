#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Command, CommandOutcome, CommandRequest, MatchView, ReplaceRequest, apply_command,
    replace_from, view_of,
};
use padel_score_server::db;
use padel_score_server::store::{EventState, NewMatch, Store, StoredEvent, StoredMatch, Teams};
use padel_scoring::{
    Current, DecidingSet, DeuceRule, ErrorKind, Event, Format, Player, PointCall, SetRules,
    SetsFormat, Start, Team,
};

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
    apply_command(s, m, req, NOW + 1).await.unwrap()
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
    let v = conflict(replace_from(&s, &m, req("r1", 1, 0), NOW).await.unwrap());
    assert_eq!(v.revision, 2);
    // base_seq beyond head_seq, or negative, is a conflict too.
    conflict(replace_from(&s, &m, req("r2", 2, 3), NOW).await.unwrap());
    conflict(replace_from(&s, &m, req("r3", 2, -1), NOW).await.unwrap());
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
        run(&s, &m, live("c2", vec![point(Team::A); 201])).await,
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
