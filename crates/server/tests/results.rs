#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::commands::{
    Authority, Command, CommandOutcome, CommandRequest, MatchView, ReplaceRequest, apply_command,
    replace_from,
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
        apply_command(
            s,
            m,
            live("finish", points(team, 24)),
            now,
            Authority::Scorer,
        )
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
        apply_command(
            &s,
            &m,
            live("c1", points(Team::A, 23)),
            NOW + 1,
            Authority::Scorer,
        )
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
            Authority::Scorer,
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
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    applied(
        apply_command(
            &s,
            &m,
            live("again", points(Team::A, 1)),
            NOW + 7,
            Authority::Scorer,
        )
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
            Authority::Scorer,
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
            Authority::Scorer,
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
        apply_command(&s, &m, live("timed", commands), NOW + 2, Authority::Scorer)
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
    let outcome = apply_command(
        &s,
        &m,
        live("late", points(Team::B, 1)),
        NOW + 9,
        Authority::Scorer,
    )
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
        apply_command(
            &s,
            &open,
            live("c1", points(Team::A, 3)),
            NOW + 6,
            Authority::Scorer,
        )
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

/// Drop every result row, as if the matches had finished before `match_results` existed.
async fn forget_results(pool: &SqlitePool) {
    sqlx::query("DELETE FROM match_results")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn backfill_revisits_only_matches_that_moved() {
    let (s, pool) = setup().await;
    let done = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &done, Team::B, NOW + 5).await;
    forget_results(&pool).await;
    assert_eq!(results::backfill(&s).await.unwrap(), 1);

    // Nothing moved since: the match is not replayed, so a row removed behind its back stays gone.
    forget_results(&pool).await;
    assert_eq!(results::backfill(&s).await.unwrap(), 0);
    assert_eq!(count_rows(&pool).await, 0);
}

#[tokio::test]
async fn backfill_picks_up_a_match_that_finishes_after_a_run() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    applied(
        apply_command(
            &s,
            &m,
            live("c1", points(Team::A, 3)),
            NOW + 1,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    // Unfinished: checked, no row.
    assert_eq!(results::backfill(&s).await.unwrap(), 0);

    applied(
        apply_command(
            &s,
            &m,
            live("c2", points(Team::A, 21)),
            NOW + 5,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    forget_results(&pool).await;
    assert_eq!(results::backfill(&s).await.unwrap(), 1);
    assert_eq!(result_rows(&pool, &m.id).await[0].finished_at, NOW + 5);
}

#[tokio::test]
async fn backfill_follows_a_retracted_result() {
    let (s, pool) = setup().await;
    let m = new_match(&s, one_set(), Some(OWNER)).await;
    let v = finish(&s, &m, Team::A, NOW + 5).await;
    forget_results(&pool).await;
    assert_eq!(results::backfill(&s).await.unwrap(), 1);

    // Undo retracts the row and moves the revision; backfill looks again and adds nothing back.
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
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(count_rows(&pool).await, 0);
    assert_eq!(results::backfill(&s).await.unwrap(), 0);
    assert_eq!(count_rows(&pool).await, 0);

    // Finished again, the row is rebuilt by the command; a backfill leaves it alone.
    applied(
        apply_command(
            &s,
            &m,
            live("again", points(Team::A, 1)),
            NOW + 7,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(count_rows(&pool).await, 1);
    assert_eq!(results::backfill(&s).await.unwrap(), 0);
    assert_eq!(count_rows(&pool).await, 1);
}

// ---- history and records ----

use padel_score_server::players::{PlayerChange, PlayerSlots};
use padel_score_server::results::{Cursor, Outcome, Record};

fn timed() -> Format {
    Format::Timed {
        deuce: DeuceRule::GoldenPoint,
        draws_allowed: true,
    }
}

async fn player(s: &Store, owner: &str, name: &str) -> String {
    s.create_or_get_player(owner, name, NOW).await.unwrap().0.id
}

/// A match of `owner` with Ana and Marta on team A and Rui and João on team B, with the
/// given slots picked from the list.
async fn match_with(s: &Store, format: Format, owner: &str, slots: PlayerSlots) -> StoredMatch {
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
                a: ["Ana".into(), "Marta".into()],
                b: ["Rui".into(), "João".into()],
            },
            owner_user_id: Some(owner.to_owned()),
            players: slots,
        },
        NOW,
    )
    .await
    .unwrap()
}

/// A 2-2 timed draw.
async fn draw(s: &Store, m: &StoredMatch, now: i64) {
    let mut commands = Vec::new();
    for team in [Team::A, Team::B, Team::A, Team::B] {
        commands.extend(points(team, 4));
    }
    commands.push(Command::TimeUp);
    applied(
        apply_command(s, m, live("timed", commands), now, Authority::Scorer)
            .await
            .unwrap(),
    );
}

struct Four {
    ana: String,
    marta: String,
    rui: String,
    joao: String,
}

async fn four(s: &Store) -> Four {
    Four {
        ana: player(s, OWNER, "Ana").await,
        marta: player(s, OWNER, "Marta").await,
        rui: player(s, OWNER, "Rui").await,
        joao: player(s, OWNER, "João").await,
    }
}

fn all_four(p: &Four) -> PlayerSlots {
    PlayerSlots {
        a: [Some(p.ana.clone()), Some(p.marta.clone())],
        b: [Some(p.rui.clone()), Some(p.joao.clone())],
    }
}

fn rec(wins: u32, losses: u32, draws: u32) -> Record {
    Record {
        wins,
        losses,
        draws,
    }
}

#[tokio::test]
async fn history_lists_finished_owned_matches_newest_first() {
    let (s, _pool) = setup().await;
    let older = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &older, Team::A, NOW + 5).await;
    let newer = new_match(&s, one_set(), Some(OWNER)).await;
    finish(&s, &newer, Team::B, NOW + 8).await;
    let open = new_match(&s, one_set(), Some(OWNER)).await;
    applied(
        apply_command(
            &s,
            &open,
            live("c1", points(Team::A, 3)),
            NOW + 9,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    let anon = new_match(&s, one_set(), None).await;
    finish(&s, &anon, Team::A, NOW + 10).await;
    let theirs = new_match(&s, one_set(), Some("user-rui")).await;
    finish(&s, &theirs, Team::A, NOW + 11).await;

    let (items, next) = s.history(OWNER, None, 30).await.unwrap();
    assert_eq!(next, None);
    let ids: Vec<&str> = items.iter().map(|e| e.match_id.as_str()).collect();
    assert_eq!(ids, vec![newer.id.as_str(), older.id.as_str()]);
    let first = &items[0];
    assert_eq!(first.finished_at, NOW + 8);
    assert_eq!(first.winner, Some(Team::B));
    assert_eq!(first.teams, newer.teams);
    assert_eq!(first.spectator_token, newer.spectator_token);
    assert_eq!(first.players, PlayerSlots::default());
    assert_eq!(first.score.games, [0, 6]);

    let json = serde_json::to_value(first).unwrap();
    assert_eq!(json["winner"], "b");
    assert_eq!(
        json["players"],
        serde_json::json!({"a": [null, null], "b": [null, null]})
    );
    assert_eq!(json["teams"]["a"][0], "Ana");
}

#[tokio::test]
async fn pagination_ties_on_finished_at() {
    let (s, _pool) = setup().await;
    let mut all = Vec::new();
    for _ in 0..5 {
        let m = new_match(&s, one_set(), Some(OWNER)).await;
        finish(&s, &m, Team::A, NOW + 5).await;
        all.push(m.id);
    }
    let mut seen = Vec::new();
    let mut before: Option<Cursor> = None;
    let mut pages = 0;
    loop {
        let (items, next) = s.history(OWNER, before.as_ref(), 2).await.unwrap();
        pages += 1;
        assert!(items.len() <= 2);
        seen.extend(items.into_iter().map(|e| e.match_id));
        match next {
            Some(cursor) => {
                // The cursor survives its own text form.
                let text = cursor.to_string();
                assert_eq!(Cursor::parse(&text), Some(cursor.clone()));
                before = Some(cursor);
            }
            None => break,
        }
        assert!(pages < 10, "pagination does not end");
    }
    assert_eq!(pages, 3);
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5);
    all.sort();
    assert_eq!(sorted, all);
    // Newest first: on a tie, the larger match id first.
    let mut desc = all.clone();
    desc.reverse();
    assert_eq!(seen, desc);
}

#[test]
fn cursor_parse_rejects_garbage() {
    assert_eq!(Cursor::parse(""), None);
    assert_eq!(Cursor::parse("abc"), None);
    assert_eq!(Cursor::parse("12"), None);
    assert_eq!(Cursor::parse("12."), None);
    assert_eq!(Cursor::parse("x.abc"), None);
    assert_eq!(
        Cursor::parse("12.abc"),
        Some(Cursor {
            finished_at: 12,
            match_id: "abc".into()
        })
    );
    assert_eq!(
        Cursor {
            finished_at: 12,
            match_id: "abc".into()
        }
        .to_string(),
        "12.abc"
    );
}

#[tokio::test]
async fn record_counts_both_players_of_a_team() {
    let (s, _pool) = setup().await;
    let p = four(&s).await;
    let m = match_with(&s, one_set(), OWNER, all_four(&p)).await;
    finish(&s, &m, Team::A, NOW + 5).await;
    let records = s.records(OWNER).await.unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(records[&p.ana], rec(1, 0, 0));
    assert_eq!(records[&p.marta], rec(1, 0, 0));
    assert_eq!(records[&p.rui], rec(0, 1, 0));
    assert_eq!(records[&p.joao], rec(0, 1, 0));
    assert_eq!(s.record(OWNER, &p.ana).await.unwrap(), rec(1, 0, 0));
    assert_eq!(s.record(OWNER, &p.joao).await.unwrap(), rec(0, 1, 0));
    // Another owner sees nothing of them.
    assert!(s.records("user-rui").await.unwrap().is_empty());
    assert_eq!(
        s.record("user-rui", &p.ana).await.unwrap(),
        Record::default()
    );
    // An unfinished match counts for nobody.
    let open = match_with(&s, one_set(), OWNER, all_four(&p)).await;
    applied(
        apply_command(
            &s,
            &open,
            live("c1", points(Team::B, 3)),
            NOW + 6,
            Authority::Scorer,
        )
        .await
        .unwrap(),
    );
    assert_eq!(s.record(OWNER, &p.rui).await.unwrap(), rec(0, 1, 0));
}

#[tokio::test]
async fn draw_counts_as_draw() {
    let (s, _pool) = setup().await;
    let p = four(&s).await;
    let m = match_with(&s, timed(), OWNER, all_four(&p)).await;
    draw(&s, &m, NOW + 5).await;
    let records = s.records(OWNER).await.unwrap();
    for id in [&p.ana, &p.marta, &p.rui, &p.joao] {
        assert_eq!(records[id], rec(0, 0, 1));
    }
    let (items, _) = s
        .player_history(OWNER, &p.rui, None, 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].outcome, Outcome::Draw);
    assert_eq!(items[0].entry.winner, None);
}

#[tokio::test]
async fn rename_keeps_record_and_match_names() {
    let (s, _pool) = setup().await;
    let p = four(&s).await;
    let m = match_with(&s, one_set(), OWNER, all_four(&p)).await;
    finish(&s, &m, Team::A, NOW + 5).await;
    s.update_player(
        OWNER,
        &p.ana,
        PlayerChange {
            name: Some("Ana Sofia".into()),
            archived: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(s.record(OWNER, &p.ana).await.unwrap(), rec(1, 0, 0));
    let (items, _) = s.history(OWNER, None, 30).await.unwrap();
    assert_eq!(items[0].teams.a[0], "Ana");
    assert_eq!(items[0].players.a[0].as_deref(), Some(p.ana.as_str()));
    // Archived players keep their record too.
    s.update_player(
        OWNER,
        &p.ana,
        PlayerChange {
            name: None,
            archived: Some(true),
        },
    )
    .await
    .unwrap();
    assert_eq!(s.record(OWNER, &p.ana).await.unwrap(), rec(1, 0, 0));
}

#[tokio::test]
async fn player_history_marks_side_and_outcome() {
    let (s, _pool) = setup().await;
    let p = four(&s).await;
    // Ana on A with a free-text partner, Rui on B with a free-text partner.
    let slots = PlayerSlots {
        a: [Some(p.ana.clone()), None],
        b: [Some(p.rui.clone()), None],
    };
    let won = match_with(&s, one_set(), OWNER, slots.clone()).await;
    finish(&s, &won, Team::A, NOW + 5).await;
    let lost = match_with(&s, one_set(), OWNER, slots.clone()).await;
    finish(&s, &lost, Team::B, NOW + 6).await;
    // A match without Ana is not in her history.
    let other = match_with(
        &s,
        one_set(),
        OWNER,
        PlayerSlots {
            a: [None, Some(p.marta.clone())],
            b: [None, None],
        },
    )
    .await;
    finish(&s, &other, Team::A, NOW + 7).await;

    let (items, next) = s
        .player_history(OWNER, &p.ana, None, 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next, None);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].entry.match_id, lost.id);
    assert_eq!(items[0].side, Team::A);
    assert_eq!(items[0].outcome, Outcome::Loss);
    assert_eq!(items[1].entry.match_id, won.id);
    assert_eq!(items[1].outcome, Outcome::Win);
    assert_eq!(items[1].entry.players, slots);

    let (items, _) = s
        .player_history(OWNER, &p.rui, None, 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].side, Team::B);
    assert_eq!(items[0].outcome, Outcome::Win);
    assert_eq!(items[1].outcome, Outcome::Loss);

    let json = serde_json::to_value(&items[0]).unwrap();
    assert_eq!(json["side"], "b");
    assert_eq!(json["outcome"], "win");
    assert_eq!(json["match_id"], lost.id.as_str());
    assert!(json.get("entry").is_none());

    // Paging works on a player's history too.
    let (page, next) = s
        .player_history(OWNER, &p.ana, None, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.len(), 1);
    let (page2, next2) = s
        .player_history(OWNER, &p.ana, next.as_ref(), 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page2[0].entry.match_id, won.id);
    assert_eq!(next2, None);

    // Another owner's player: None.
    assert!(
        s.player_history("user-rui", &p.ana, None, 30)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        s.player_history(OWNER, "nope", None, 30)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn placings_are_returned() {
    let (s, pool) = setup().await;
    let ana = player(&s, OWNER, "Ana").await;
    assert!(s.placings(OWNER, &ana).await.unwrap().is_empty());
    for (event, kind, place, place_to, field, at) in [
        ("ev-old", "tournament", 1, 1, 8, NOW),
        ("ev-new", "non_stop", 3, 4, 12, NOW + 50),
    ] {
        sqlx::query(
            "INSERT INTO competitions (id, owner_user_id, name, kind, format, spectator_token, \
             created_at) VALUES (?, ?, ?, ?, '{}', ?, 1)",
        )
        .bind(event)
        .bind(OWNER)
        .bind(format!("Event {event}"))
        .bind(kind)
        .bind(format!("tok-{event}"))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO placings (player_id, event_id, event_kind, place, place_to, field_size, \
             finished_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ana)
        .bind(event)
        .bind(kind)
        .bind(place)
        .bind(place_to)
        .bind(field)
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
    }
    let placings = s.placings(OWNER, &ana).await.unwrap();
    assert_eq!(placings.len(), 2);
    assert_eq!(placings[0].event_id, "ev-new");
    assert_eq!(placings[0].event_name, "Event ev-new");
    assert_eq!(placings[0].event_kind, "non_stop");
    assert_eq!(placings[0].place, 3);
    assert_eq!(placings[0].place_to, 4);
    assert_eq!(placings[0].field_size, 12);
    assert_eq!(placings[0].finished_at, NOW + 50);
    assert_eq!(placings[1].event_id, "ev-old");
}

#[tokio::test]
async fn placings_exclude_other_coordinators_events() {
    let (s, pool) = setup().await;
    let ana = player(&s, OWNER, "Ana").await;
    for (event, owner, at) in [("ev-mine", OWNER, NOW), ("ev-theirs", "user-rui", NOW + 50)] {
        sqlx::query(
            "INSERT INTO competitions (id, owner_user_id, name, kind, format, spectator_token, \
             created_at) VALUES (?, ?, ?, 'tournament', '{}', ?, 1)",
        )
        .bind(event)
        .bind(owner)
        .bind(format!("Event {event}"))
        .bind(format!("tok-{event}"))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO placings (player_id, event_id, event_kind, place, place_to, field_size, \
             finished_at) VALUES (?, ?, 'tournament', 1, 1, 8, ?)",
        )
        .bind(&ana)
        .bind(event)
        .bind(at)
        .execute(&pool)
        .await
        .unwrap();
    }
    let ids: Vec<String> = s
        .placings(OWNER, &ana)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.event_id)
        .collect();
    assert_eq!(ids, ["ev-mine"]);
}
