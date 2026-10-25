#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod support;

use padel_scoring::DeuceRule::*;
use padel_scoring::*;
use support::{ev, start_a, timed};

fn play(format: &Format, s: &str) -> Score {
    replay(format, &start_a(), &ev(s)).unwrap()
}

fn p(team: Team, index: u8) -> Player {
    Player { team, index }
}

fn draws() -> Format {
    timed(GoldenPoint, true)
}

fn no_draws() -> Format {
    timed(GoldenPoint, false)
}

const LOVE_ALL: GameScore = GameScore {
    a: PointCall::Love,
    b: PointCall::Love,
    deuces: 0,
    deciding_point: false,
};

#[test]
fn fresh_timed_match_is_a_game_in_progress() {
    let s = play(&draws(), "");
    assert_eq!(s.current, Current::Game(LOVE_ALL));
    assert_eq!(s.status, Status::InProgress);
    assert_eq!(s.games, [0, 0]);
    assert!(s.sets.is_empty());
    assert_eq!(s.sets_won, [0, 0]);
    assert!(!s.time_up);
    assert!(!s.overtime);
}

#[test]
fn games_accumulate_past_six() {
    let s = play(&draws(), &"aaaa".repeat(7));
    assert_eq!(s.games, [7, 0]);
    assert_eq!(s.status, Status::InProgress);
    assert_eq!(s.current, Current::Game(LOVE_ALL));
    assert!(s.sets.is_empty());
    assert_eq!(s.sets_won, [0, 0]);
}

#[test]
fn time_up_discards_game_in_progress() {
    let s = play(&draws(), "aaaa aa t");
    assert_eq!(s.games, [1, 0]);
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(s.current, Current::None);
    assert!(s.time_up);
    assert!(!s.overtime);
}

#[test]
fn time_up_with_games_differing_finishes_without_draws_too() {
    let s = play(&no_draws(), "bbbb bb t");
    assert_eq!(s.games, [0, 1]);
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::B)
        }
    );
    assert!(!s.overtime);
}

#[test]
fn level_with_draws_allowed_is_draw() {
    let s = play(&draws(), "aaaa bbbb t");
    assert_eq!(s.status, Status::Finished { winner: None });
    assert_eq!(s.games, [1, 1]);
    assert_eq!(s.current, Current::None);
    assert!(s.time_up);
    assert!(!s.overtime);
}

#[test]
fn level_without_draws_goes_to_overtime() {
    let s = play(&no_draws(), "aaaa bbbb t");
    assert!(s.overtime);
    assert!(s.time_up);
    assert_eq!(s.status, Status::InProgress);
    assert_eq!(s.current, Current::Game(LOVE_ALL));
    assert_eq!(s.games, [1, 1]);

    let s = play(&no_draws(), "aaaa bbbb t bbbb");
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::B)
        }
    );
    assert_eq!(s.games, [1, 2]);
    assert_eq!(s.current, Current::None);
    assert!(s.overtime);
    assert!(s.time_up);
}

#[test]
fn overtime_is_a_single_game_with_the_same_deuce_rule() {
    // Golden point: at 40-40 in overtime the next point wins the match.
    let s = play(&no_draws(), "aaaa bbbb t ababab");
    assert_eq!(s.status, Status::InProgress);
    assert!(matches!(
        s.current,
        Current::Game(GameScore {
            deciding_point: true,
            ..
        })
    ));
    assert_eq!(s.side, ServeSide::ReceiverChooses);
    let s = play(&no_draws(), "aaaa bbbb t ababab a");
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(s.games, [2, 1]);

    // Advantage: 40-40 then advantage, no decision yet.
    let s = play(&timed(Advantage, false), "aaaa bbbb t abababa");
    assert_eq!(s.status, Status::InProgress);
}

#[test]
fn overtime_server_is_the_discarded_games_server() {
    // Two games played: A0 then B0 served; A1 serves game 3, which time-up discards.
    let before = play(&no_draws(), "aaaa bbbb aa");
    assert_eq!(before.server, p(Team::A, 1));
    let s = play(&no_draws(), "aaaa bbbb aa t");
    assert!(s.overtime);
    assert_eq!(s.server, p(Team::A, 1));
    assert_eq!(s.side, ServeSide::Right);
    assert_eq!(s.current, Current::Game(LOVE_ALL));
}

#[test]
fn timed_serving_follows_rotation_and_side() {
    let f = draws();
    assert_eq!(play(&f, "").server, p(Team::A, 0));
    assert_eq!(play(&f, "").side, ServeSide::Right);
    assert_eq!(play(&f, "a").side, ServeSide::Left);
    assert_eq!(play(&f, "aaaa").server, p(Team::B, 0));
    assert_eq!(play(&f, "aaaa aaaa").server, p(Team::A, 1));
    assert_eq!(play(&f, "aaaa aaaa aaaa").server, p(Team::B, 1));
    assert_eq!(play(&f, &"aaaa".repeat(4)).server, p(Team::A, 0));
    assert_eq!(play(&f, &"aaaa".repeat(9)).server, p(Team::B, 0));
}

#[test]
fn timed_change_ends_counts_all_games_in_match() {
    let f = draws();
    assert!(!play(&f, "").change_ends);
    assert!(play(&f, "aaaa").change_ends);
    assert!(!play(&f, "aaaa a").change_ends);
    assert!(!play(&f, "aaaa bbbb").change_ends);
    // No sets: games 7 and 13 are odd totals in the match, so ends change.
    assert!(play(&f, &"aaaa".repeat(7)).change_ends);
    assert!(!play(&f, &"aaaa".repeat(12)).change_ends);
    assert!(play(&f, &"aaaa".repeat(13)).change_ends);
}

#[test]
fn no_change_ends_after_match_point_timed() {
    // One game played (odd) changes ends, but time-up then ends the match.
    assert!(play(&draws(), "aaaa").change_ends);
    let s = play(&draws(), "aaaa t");
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert!(!s.change_ends);
    // The overtime game is the 3rd game (odd) but it ends the match.
    let s = play(&no_draws(), "aaaa bbbb t aaaa");
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert!(!s.change_ends);
}

#[test]
fn time_up_into_overtime_does_not_change_ends() {
    // Regression guard: time-up never changes ends itself. Four games (an even number) are played,
    // so nothing before the time-up changed ends either, and the overtime starts without a change.
    let s = play(&no_draws(), "aaaa bbbb aaaa bbbb t");
    assert!(s.overtime);
    assert!(!s.change_ends);
}

#[test]
fn second_time_up_rejected() {
    let mut m = MatchState::new(no_draws(), start_a()).unwrap();
    for e in ev("aaaa bbbb t") {
        m.apply(e).unwrap();
    }
    let before = m.clone();
    assert_eq!(m.apply(Event::TimeUp), Err(ErrorKind::AlreadyTimeUp));
    assert_eq!(m, before);
    assert_eq!(
        replay(&no_draws(), &start_a(), &ev("aaaa bbbb t t")),
        Err(ReplayError {
            index: 9,
            kind: ErrorKind::AlreadyTimeUp
        })
    );
}

#[test]
fn time_up_after_finish_is_match_over() {
    assert_eq!(
        replay(&draws(), &start_a(), &ev("aaaa t t")),
        Err(ReplayError {
            index: 5,
            kind: ErrorKind::MatchOver
        })
    );
    assert_eq!(
        replay(&no_draws(), &start_a(), &ev("aaaa bbbb t aaaa t")),
        Err(ReplayError {
            index: 13,
            kind: ErrorKind::MatchOver
        })
    );
}

#[test]
fn timed_point_after_finish_is_rejected() {
    let mut m = MatchState::new(draws(), start_a()).unwrap();
    for e in ev("aaaa bbbb t") {
        m.apply(e).unwrap();
    }
    let before = m.clone();
    assert_eq!(m.apply(Event::Point(Team::A)), Err(ErrorKind::MatchOver));
    assert_eq!(m, before);
    assert_eq!(
        replay(&no_draws(), &start_a(), &ev("aaaa bbbb t bbbb a")),
        Err(ReplayError {
            index: 13,
            kind: ErrorKind::MatchOver
        })
    );
}

#[test]
fn timed_score_round_trips_through_serde() {
    let mut m = MatchState::new(no_draws(), start_a()).unwrap();
    for e in ev("aaaa bbbb t ab") {
        m.apply(e).unwrap();
    }
    let score = m.score();
    let json = serde_json::to_string(&score).unwrap();
    let back: Score = serde_json::from_str(&json).unwrap();
    assert_eq!(back, score);
}
