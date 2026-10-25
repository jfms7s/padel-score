#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod support;

use padel_scoring::DecidingSet::*;
use padel_scoring::DeuceRule::*;
use padel_scoring::SetsFormat::*;
use padel_scoring::*;
use support::{bo3, ev, games, sets, start_a};

fn play(format: &Format, s: &str) -> Score {
    replay(format, &start_a(), &ev(s)).unwrap()
}

#[test]
fn six_four_wins_a_set() {
    let s = play(&bo3(), &games(6, 4));
    assert_eq!(s.sets_won, [1, 0]);
    assert_eq!(
        s.sets[0],
        SetScore {
            a: 6,
            b: 4,
            tiebreak: None
        }
    );
    assert_eq!(s.games, [0, 0]);
    assert_eq!(s.status, Status::InProgress);
}

#[test]
fn six_five_is_not_a_set() {
    let s = play(&bo3(), &games(6, 5));
    assert_eq!(s.sets_won, [0, 0]);
    assert_eq!(s.games, [6, 5]);
    assert!(matches!(s.current, Current::Game(_)));
}

#[test]
fn seven_five_wins_a_set() {
    let s = play(&bo3(), &(games(5, 5) + "aaaa" + "aaaa"));
    assert_eq!(s.sets_won, [1, 0]);
    assert_eq!(
        s.sets[0],
        SetScore {
            a: 7,
            b: 5,
            tiebreak: None
        }
    );
}

#[test]
fn tiebreak_at_six_all() {
    let at_six = play(&bo3(), &games(6, 6));
    assert_eq!(
        at_six.current,
        Current::TieBreak {
            a: 0,
            b: 0,
            super_tiebreak: false
        }
    );
    let s = play(&bo3(), &(games(6, 6) + "aaaaaaa"));
    assert_eq!(s.sets_won, [1, 0]);
    assert_eq!(
        s.sets[0],
        SetScore {
            a: 7,
            b: 6,
            tiebreak: Some((7, 0))
        }
    );
    assert!(matches!(s.current, Current::Game(_)));
}

#[test]
fn best_of_three_ends_at_two_sets() {
    let s = play(&bo3(), &(games(6, 0) + &games(6, 0)));
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(s.current, Current::None);
    assert_eq!(s.sets_won, [2, 0]);
    assert_eq!(s.sets.len(), 2);
}

#[test]
fn best_of_three_full_deciding_set() {
    let s = play(&bo3(), &(games(6, 0) + &games(0, 6) + &games(6, 6)));
    assert_eq!(s.sets_won, [1, 1]);
    assert_eq!(
        s.current,
        Current::TieBreak {
            a: 0,
            b: 0,
            super_tiebreak: false
        }
    );
    let s = play(
        &bo3(),
        &(games(6, 0) + &games(0, 6) + &games(6, 6) + "bbbbbbb"),
    );
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::B)
        }
    );
    assert_eq!(
        s.sets[2],
        SetScore {
            a: 6,
            b: 7,
            tiebreak: Some((0, 7))
        }
    );
}

#[test]
fn one_set_match_ends_after_one_set() {
    let s = play(&sets(Advantage, One, FullSet), &games(4, 6));
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::B)
        }
    );
    assert_eq!(s.sets_won, [0, 1]);
}

#[test]
fn super_tiebreak_only_in_deciding_set() {
    let f = sets(GoldenPoint, BestOf3, SuperTieBreak);
    let s = play(&f, &games(6, 6));
    assert_eq!(
        s.current,
        Current::TieBreak {
            a: 0,
            b: 0,
            super_tiebreak: false
        }
    );
    let two_sets = games(6, 0) + &games(0, 6);
    let s = play(&f, &two_sets);
    assert_eq!(s.sets_won, [1, 1]);
    assert_eq!(
        s.current,
        Current::TieBreak {
            a: 0,
            b: 0,
            super_tiebreak: true
        }
    );
    let s = play(&f, &(two_sets.clone() + "aaaaaaaaa"));
    assert_eq!(s.status, Status::InProgress);
    let s = play(&f, &(two_sets + "aaaaaaaaaa"));
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(
        s.sets[2],
        SetScore {
            a: 1,
            b: 0,
            tiebreak: Some((10, 0))
        }
    );
    assert_eq!(s.current, Current::None);
}

#[test]
fn point_after_match_over_is_rejected() {
    let mut m = MatchState::new(bo3(), start_a()).unwrap();
    for e in ev(&(games(6, 0) + &games(6, 0))) {
        m.apply(e).unwrap();
    }
    let before = m.score();
    assert_eq!(m.apply(Event::Point(Team::B)), Err(ErrorKind::MatchOver));
    assert_eq!(m.score(), before);
    assert_eq!(
        replay(&bo3(), &start_a(), &ev(&(games(6, 0) + &games(6, 0) + "b"))).unwrap_err(),
        ReplayError {
            index: 48,
            kind: ErrorKind::MatchOver
        }
    );
}

#[test]
fn time_up_in_sets_format_is_rejected() {
    let mut m = MatchState::new(bo3(), start_a()).unwrap();
    m.apply(Event::Point(Team::A)).unwrap();
    let before = m.score();
    assert_eq!(m.apply(Event::TimeUp), Err(ErrorKind::NotTimed));
    assert_eq!(m.score(), before);
}

#[test]
fn invalid_rules_are_rejected() {
    let bad = sets(GoldenPoint, One, SuperTieBreak);
    assert_eq!(
        MatchState::new(bad, start_a()).unwrap_err(),
        RulesError::SuperTieBreakNeedsMultipleSets
    );
    assert_eq!(
        replay(&bad, &start_a(), &[]).unwrap_err(),
        ReplayError {
            index: 0,
            kind: ErrorKind::InvalidRules(RulesError::SuperTieBreakNeedsMultipleSets)
        }
    );
    let bad_start = Start {
        first_server: Player {
            team: Team::A,
            index: 2,
        },
        other_first_server: 0,
    };
    assert_eq!(
        MatchState::new(bo3(), bad_start).unwrap_err(),
        RulesError::InvalidPlayerIndex
    );
}

#[test]
fn placeholder_serve_fields_until_task_5() {
    let s = play(&bo3(), "");
    assert_eq!(s.server, start_a().first_server);
    assert_eq!(s.side, ServeSide::Right);
    assert_eq!(s.sets.len(), 1);
    assert_eq!(s.status, Status::InProgress);
}
