mod support;

use padel_scoring::*;
use support::ev;

fn play(target: u8, s: &str) -> (TieBreak, Option<Team>) {
    let mut tb = TieBreak::new(target);
    let mut winner = None;
    for e in ev(s) {
        if let Event::Point(t) = e {
            winner = tb.point(t);
        }
    }
    (tb, winner)
}

fn rep(c: char, n: usize) -> String {
    std::iter::repeat_n(c, n).collect()
}

#[test]
fn constants_are_seven_and_ten() {
    assert_eq!(TIEBREAK_TO, 7);
    assert_eq!(SUPER_TIEBREAK_TO, 10);
}

#[test]
fn seven_love_wins() {
    let (tb, w) = play(TIEBREAK_TO, "aaaaaa");
    assert_eq!(w, None);
    assert_eq!(tb.points(), (6, 0));
    let (tb, w) = play(TIEBREAK_TO, "aaaaaaa");
    assert_eq!(w, Some(Team::A));
    assert_eq!(tb.points(), (7, 0));
    assert_eq!(tb.points_played(), 7);
}

#[test]
fn needs_two_clear() {
    let (_, w) = play(TIEBREAK_TO, "aaaaaabbbbbb");
    assert_eq!(w, None);
    let (_, w) = play(TIEBREAK_TO, "aaaaaabbbbbba");
    assert_eq!(w, None);
    let (tb, w) = play(TIEBREAK_TO, "aaaaaabbbbbbaa");
    assert_eq!(w, Some(Team::A));
    assert_eq!(tb.points(), (8, 6));
}

#[test]
fn seven_six_is_not_a_win_and_b_can_win() {
    let (_, w) = play(TIEBREAK_TO, "aaaaaabbbbbbbb");
    assert_eq!(w, Some(Team::B));
    let (_, w) = play(TIEBREAK_TO, "aaaaaabbbbbba");
    assert_eq!(w, None);
}

#[test]
fn super_to_ten() {
    let base = format!("{}{}", rep('a', 9), rep('b', 9));
    let (_, w) = play(SUPER_TIEBREAK_TO, &base);
    assert_eq!(w, None);
    let (_, w) = play(SUPER_TIEBREAK_TO, &format!("{base}a"));
    assert_eq!(w, None, "10-9 is not a win");
    let (tb, w) = play(SUPER_TIEBREAK_TO, &format!("{base}aa"));
    assert_eq!(w, Some(Team::A));
    assert_eq!(tb.points(), (11, 9));
    let (_, w) = play(SUPER_TIEBREAK_TO, &rep('b', 10));
    assert_eq!(w, Some(Team::B));
}

#[test]
fn points_after_win_do_not_change_tiebreak() {
    let (mut tb, w) = play(TIEBREAK_TO, "aaaaaaa");
    assert_eq!(w, Some(Team::A));
    assert_eq!(tb.point(Team::B), Some(Team::A));
    assert_eq!(tb.point(Team::A), Some(Team::A));
    assert_eq!(tb.points(), (7, 0));
    assert_eq!(tb.points_played(), 7);
}

#[test]
fn long_rally_does_not_overflow() {
    let mut tb = TieBreak::new(TIEBREAK_TO);
    for _ in 0..1000 {
        assert_eq!(tb.point(Team::A), None);
        assert_eq!(tb.point(Team::B), None);
    }
    assert_eq!(tb.points(), (1000, 1000));
}
