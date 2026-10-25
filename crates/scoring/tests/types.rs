#![allow(clippy::unwrap_used)]

mod support;

use padel_scoring::*;
use support::*;

#[test]
fn format_round_trips_as_snake_case_json() {
    let f = sets(
        DeuceRule::StarPoint,
        SetsFormat::BestOf3,
        DecidingSet::SuperTieBreak,
    );
    let json = serde_json::to_string(&f).unwrap();
    assert_eq!(
        json,
        r#"{"sets":{"deuce":"star_point","sets":"best_of3","deciding":"super_tie_break"}}"#
    );
    assert_eq!(serde_json::from_str::<Format>(&json).unwrap(), f);
}

#[test]
fn one_set_with_super_tiebreak_is_invalid() {
    assert_eq!(
        sets(
            DeuceRule::Advantage,
            SetsFormat::One,
            DecidingSet::SuperTieBreak
        )
        .validate(),
        Err(RulesError::SuperTieBreakNeedsMultipleSets)
    );
    assert_eq!(timed(DeuceRule::GoldenPoint, false).validate(), Ok(()));
}

#[test]
fn ev_parses_shorthand() {
    assert_eq!(
        ev("ab t"),
        vec![Event::Point(Team::A), Event::Point(Team::B), Event::TimeUp]
    );
}
