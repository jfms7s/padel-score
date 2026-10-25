mod support;

use padel_scoring::DeuceRule::*;
use padel_scoring::*;
use support::ev;

fn play(rule: DeuceRule, s: &str) -> (Option<Team>, GameScore) {
    let mut game = Game::new(rule);
    let mut winner = None;
    for e in ev(s) {
        if let Event::Point(t) = e {
            winner = winner.or(game.point(t));
        }
    }
    (winner, game.score())
}

#[test]
fn four_straight_points_win() {
    assert_eq!(play(Advantage, "aaaa").0, Some(Team::A));
}

#[test]
fn advantage_needs_two_clear() {
    let (w, s) = play(Advantage, "aaabbb a");
    assert_eq!(w, None);
    assert_eq!((s.a, s.b), (PointCall::Advantage, PointCall::Forty));
    let (w, s) = play(Advantage, "aaabbb ab");
    assert_eq!(w, None);
    assert_eq!(s.deuces, 2);
    assert!(!s.deciding_point);
    assert_eq!(play(Advantage, "aaabbb aa").0, Some(Team::A));
}

#[test]
fn golden_point_decides_at_first_deuce() {
    let (_, s) = play(GoldenPoint, "aaabbb");
    assert!(s.deciding_point);
    assert_eq!(play(GoldenPoint, "aaabbb b").0, Some(Team::B));
}

#[test]
fn star_point_decides_on_third_deuce() {
    let (_, s) = play(StarPoint, "aaabbb ab");
    assert_eq!(s.deuces, 2);
    assert!(!s.deciding_point);
    let (w, _) = play(StarPoint, "aaabbb ab a");
    assert_eq!(w, None);
    let (_, s) = play(StarPoint, "aaabbb ab ab");
    assert_eq!(s.deuces, STAR_POINT_DEUCE);
    assert!(s.deciding_point);
    assert_eq!(play(StarPoint, "aaabbb ab ab b").0, Some(Team::B));
}

#[test]
fn score_calls_and_points_played() {
    let mut g = Game::new(Advantage);
    g.point(Team::A);
    g.point(Team::A);
    g.point(Team::B);
    let s = g.score();
    assert_eq!(
        (s.a, s.b, s.deuces),
        (PointCall::Thirty, PointCall::Fifteen, 0)
    );
    assert_eq!(g.points_played(), 3);
}
