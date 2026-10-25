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
        r#"{"sets":{"deuce":"star_point","sets":"best_of_3","deciding":"super_tie_break"}}"#
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

#[test]
fn start_with_bad_player_index_is_invalid() {
    let bad = Start {
        first_server: Player {
            team: Team::B,
            index: 2,
        },
        other_first_server: 0,
    };
    assert_eq!(bad.validate(), Err(RulesError::InvalidPlayerIndex));
    assert_eq!(start_a().validate(), Ok(()));
}

#[test]
fn start_rejects_bad_other_first_server() {
    let bad = Start {
        other_first_server: 2,
        ..start_a()
    };
    assert_eq!(bad.validate(), Err(RulesError::InvalidPlayerIndex));
    let ok = Start {
        other_first_server: 1,
        ..start_a()
    };
    assert_eq!(ok.validate(), Ok(()));
}

#[test]
fn sets_to_win_counts() {
    assert_eq!(SetsFormat::One.sets_to_win(), 1);
    assert_eq!(SetsFormat::BestOf3.sets_to_win(), 2);
}
