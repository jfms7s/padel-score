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

fn p(team: Team, index: u8) -> Player {
    Player { team, index }
}

/// The player after `x` in the rotation that starts with A0.
fn next(x: Player) -> Player {
    let order = serve_order(p(Team::A, 0));
    let i = order.iter().position(|&o| o == x).unwrap();
    order[(i + 1) % 4]
}

#[test]
fn order_alternates_teams() {
    assert_eq!(
        serve_order(p(Team::A, 0)),
        [p(Team::A, 0), p(Team::B, 0), p(Team::A, 1), p(Team::B, 1)]
    );
    assert_eq!(
        serve_order(p(Team::B, 1)),
        [p(Team::B, 1), p(Team::A, 0), p(Team::B, 0), p(Team::A, 1)]
    );
}

#[test]
fn server_rotates_each_game() {
    assert_eq!(play(&bo3(), "").server, p(Team::A, 0));
    assert_eq!(play(&bo3(), "aaaa").server, p(Team::B, 0));
    assert_eq!(play(&bo3(), "aaaa aaaa").server, p(Team::A, 1));
    assert_eq!(play(&bo3(), "aaaa aaaa aaaa").server, p(Team::B, 1));
    assert_eq!(play(&bo3(), "aaaa aaaa aaaa aaaa").server, p(Team::A, 0));
}

#[test]
fn server_rotation_continues_across_sets() {
    // Set 1 is 6-0 (6 games), so set 2 opens with the 7th server in rotation.
    let s = play(&bo3(), &games(6, 0));
    assert_eq!(s.sets_won, [1, 0]);
    assert_eq!(s.server, p(Team::A, 1));
}

#[test]
fn side_alternates_per_point() {
    assert_eq!(play(&bo3(), "").side, ServeSide::Right);
    assert_eq!(play(&bo3(), "a").side, ServeSide::Left);
    assert_eq!(play(&bo3(), "ab").side, ServeSide::Right);
}

#[test]
fn golden_point_receiver_chooses() {
    assert_eq!(play(&bo3(), "aaabbb").side, ServeSide::ReceiverChooses);
}

#[test]
fn advantage_deuce_keeps_alternating() {
    let f = sets(Advantage, BestOf3, FullSet);
    assert_eq!(play(&f, "aaabbb").side, ServeSide::Right);
    assert_eq!(play(&f, "aaabbba").side, ServeSide::Left);
}

#[test]
fn star_point_receiver_chooses_on_third_deuce() {
    let f = sets(StarPoint, BestOf3, FullSet);
    assert_eq!(play(&f, "aaabbb ab").side, ServeSide::Right);
    assert_eq!(play(&f, "aaabbb ab ab").side, ServeSide::ReceiverChooses);
}

#[test]
fn tiebreak_service_pattern() {
    let at_six_all = games(6, 6);
    let x = play(&bo3(), &at_six_all).server;
    let n = next(x);
    let x2 = next(n);
    let expected = [x, n, n, x2, x2];
    for (k, want) in expected.iter().enumerate() {
        let points = &"ababa"[..k];
        let s = play(&bo3(), &format!("{at_six_all}{points}"));
        assert!(matches!(s.current, Current::TieBreak { .. }));
        assert_eq!(s.server, *want, "server for tie-break point {}", k + 1);
    }
}

#[test]
fn tiebreak_side_alternates_per_point() {
    let at_six_all = games(6, 6);
    assert_eq!(play(&bo3(), &at_six_all).side, ServeSide::Right);
    assert_eq!(
        play(&bo3(), &format!("{at_six_all}a")).side,
        ServeSide::Left
    );
    assert_eq!(
        play(&bo3(), &format!("{at_six_all}ab")).side,
        ServeSide::Right
    );
}

#[test]
fn next_set_after_tiebreak() {
    let at_six_all = games(6, 6);
    let x = play(&bo3(), &at_six_all).server;
    let s = play(&bo3(), &format!("{at_six_all}aaaaaaa"));
    assert_eq!(s.sets_won, [1, 0]);
    assert_eq!(s.games, [0, 0]);
    assert_eq!(s.server, next(x));
}

#[test]
fn super_tiebreak_uses_tiebreak_service_pattern() {
    let f = sets(GoldenPoint, BestOf3, SuperTieBreak);
    let to_deciding = format!("{}{}", games(6, 0), games(0, 6));
    let x = play(&f, &to_deciding).server;
    let n = next(x);
    let x2 = next(n);
    let expected = [x, n, n, x2, x2];
    for (k, want) in expected.iter().enumerate() {
        let points = &"ababa"[..k];
        let s = play(&f, &format!("{to_deciding}{points}"));
        assert!(matches!(
            s.current,
            Current::TieBreak {
                super_tiebreak: true,
                ..
            }
        ));
        assert_eq!(
            s.server,
            *want,
            "server for super tie-break point {}",
            k + 1
        );
    }
}

#[test]
fn change_ends_after_odd_games() {
    assert!(!play(&bo3(), "").change_ends);
    assert!(!play(&bo3(), "aaa").change_ends);
    assert!(play(&bo3(), "aaaa").change_ends);
    assert!(!play(&bo3(), "aaaa a").change_ends);
    assert!(!play(&bo3(), "aaaa aaaa").change_ends);
    assert!(play(&bo3(), "aaaa aaaa aaaa").change_ends);

    let at_six_all = games(6, 6);
    assert!(!play(&bo3(), &at_six_all).change_ends);
    assert!(!play(&bo3(), &format!("{at_six_all}ababa")).change_ends);
    assert!(play(&bo3(), &format!("{at_six_all}ababab")).change_ends);
    assert!(!play(&bo3(), &format!("{at_six_all}ababab a")).change_ends);
    assert!(play(&bo3(), &format!("{at_six_all}ababab ababab")).change_ends);
}

#[test]
fn change_ends_counts_games_within_set() {
    // FIP Rule 5.1: ends change when the games played in the current set add up to odd.
    // Set 1 6-0 (6 games, even): no change at the set end; game 1 of set 2 changes ends.
    let set1 = games(6, 0);
    assert!(!play(&bo3(), &set1).change_ends);
    assert!(play(&bo3(), &format!("{set1}aaaa")).change_ends);
    assert!(!play(&bo3(), &format!("{set1}aaaa aaaa")).change_ends);
    // Set 1 6-3 (9 games, odd): ends change at the set end and again after game 1 of set 2.
    let set1 = games(6, 3);
    assert_eq!(play(&bo3(), &set1).sets_won, [1, 0]);
    assert!(play(&bo3(), &set1).change_ends);
    assert!(play(&bo3(), &format!("{set1}aaaa")).change_ends);
    assert!(!play(&bo3(), &format!("{set1}aaaa aaaa")).change_ends);
    // A 7-6 set: the tie-break is the 13th game, so ends change when it ends,
    // and again after game 1 of set 2.
    let tb_set = format!("{}aaaaaaa", games(6, 6));
    assert_eq!(play(&bo3(), &tb_set).sets_won, [1, 0]);
    assert!(play(&bo3(), &tb_set).change_ends);
    assert!(play(&bo3(), &format!("{tb_set}aaaa")).change_ends);
    assert!(!play(&bo3(), &format!("{tb_set}aaaa aaaa")).change_ends);
}

#[test]
fn super_tiebreak_changes_ends_every_six_points() {
    let f = sets(GoldenPoint, BestOf3, SuperTieBreak);
    let to_deciding = format!("{}{}", games(6, 0), games(0, 6));
    assert!(!play(&f, &format!("{to_deciding}ababa")).change_ends);
    assert!(play(&f, &format!("{to_deciding}ababab")).change_ends);
    assert!(!play(&f, &format!("{to_deciding}ababab a")).change_ends);
    assert!(play(&f, &format!("{to_deciding}ababab ababab")).change_ends);
}

#[test]
fn finished_match_snapshot_does_not_panic() {
    let f = sets(GoldenPoint, BestOf3, SuperTieBreak);
    let s = play(&f, &format!("{}{}aaaaaaaaaa", games(6, 0), games(0, 6)));
    assert_eq!(
        s.status,
        Status::Finished {
            winner: Some(Team::A)
        }
    );
    assert_eq!(s.games, [1, 0]);
}
