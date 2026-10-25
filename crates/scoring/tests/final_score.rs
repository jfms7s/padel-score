#![allow(clippy::unwrap_used)]

use padel_scoring::DecidingSet::*;
use padel_scoring::DeuceRule::*;
use padel_scoring::FinalScore::*;
use padel_scoring::SetsFormat::*;
use padel_scoring::*;

fn fmt(sets: SetsFormat, deciding: DecidingSet) -> Format {
    Format::Sets(SetRules {
        deuce: Advantage,
        sets,
        deciding,
    })
}

fn timed(draws_allowed: bool) -> Format {
    Format::Timed {
        deuce: Advantage,
        draws_allowed,
    }
}

fn s(a: u8, b: u8) -> SetScore {
    SetScore {
        a,
        b,
        tiebreak: None,
    }
}

fn stb(a: u8, b: u8, tb: (u16, u16)) -> SetScore {
    SetScore {
        a,
        b,
        tiebreak: Some(tb),
    }
}

#[test]
fn final_score_accepts_valid() {
    let full = fmt(BestOf3, FullSet);
    let sup = fmt(BestOf3, SuperTieBreak);
    assert_eq!(
        validate_final(&full, &Sets(vec![s(6, 4), s(6, 3)])),
        Ok(Some(Team::A))
    );
    assert_eq!(
        validate_final(&sup, &Sets(vec![s(6, 4), s(3, 6), stb(1, 0, (10, 7))])),
        Ok(Some(Team::A))
    );
    assert_eq!(
        validate_final(&full, &Sets(vec![s(7, 6), stb(6, 7, (5, 7)), s(6, 4)])),
        Ok(Some(Team::A))
    );
    assert_eq!(
        validate_final(&fmt(One, FullSet), &Sets(vec![s(2, 6)])),
        Ok(Some(Team::B))
    );
    let t = |a, b, o| Timed {
        a,
        b,
        overtime_winner: o,
    };
    assert_eq!(
        validate_final(&timed(false), &t(5, 3, None)),
        Ok(Some(Team::A))
    );
    assert_eq!(validate_final(&timed(true), &t(3, 3, None)), Ok(None));
    assert_eq!(
        validate_final(&timed(false), &t(3, 3, Some(Team::B))),
        Ok(Some(Team::B))
    );
}

#[test]
fn final_score_rejects_impossible() {
    let full = fmt(BestOf3, FullSet);
    let sup = fmt(BestOf3, SuperTieBreak);
    let one = fmt(One, FullSet);
    let imp = |index| Err(InvalidScore::ImpossibleSet { index });
    assert_eq!(validate_final(&one, &Sets(vec![s(6, 5)])), imp(0));
    assert_eq!(validate_final(&one, &Sets(vec![s(8, 6)])), imp(0));
    assert_eq!(
        validate_final(&sup, &Sets(vec![s(6, 4), s(3, 6), stb(1, 0, (10, 9))])),
        imp(2)
    );
    assert_eq!(
        validate_final(&full, &Sets(vec![s(6, 0), s(6, 0), s(6, 0)])),
        Err(InvalidScore::ExtraSets)
    );
    assert_eq!(
        validate_final(&full, &Sets(vec![s(6, 4)])),
        Err(InvalidScore::MatchNotFinished)
    );
    assert_eq!(
        validate_final(
            &timed(false),
            &Timed {
                a: 3,
                b: 3,
                overtime_winner: None
            }
        ),
        Err(InvalidScore::DrawNotAllowed)
    );
    assert_eq!(
        validate_final(
            &timed(false),
            &Timed {
                a: 4,
                b: 3,
                overtime_winner: Some(Team::B)
            }
        ),
        Err(InvalidScore::OvertimeWinnerWhenNotLevel)
    );
    assert_eq!(
        validate_final(
            &timed(true),
            &Timed {
                a: 3,
                b: 3,
                overtime_winner: Some(Team::B)
            }
        ),
        Err(InvalidScore::OvertimeWinnerWhenNotLevel)
    );
    assert_eq!(
        validate_final(&timed(true), &Sets(vec![s(6, 4)])),
        Err(InvalidScore::WrongFormat)
    );
    assert_eq!(
        validate_final(
            &one,
            &Timed {
                a: 1,
                b: 0,
                overtime_winner: None
            }
        ),
        Err(InvalidScore::WrongFormat)
    );
}

#[test]
fn final_score_tiebreak_rules() {
    let full = fmt(BestOf3, FullSet);
    let sup = fmt(BestOf3, SuperTieBreak);
    let imp = |index| Err(InvalidScore::ImpossibleSet { index });
    // tie-break points on a set that is not 7-6
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(6, 4, (7, 5))])),
        imp(0)
    );
    // tie-break won by the other team than the set
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (5, 7))])),
        imp(0)
    );
    // invalid tie-break points
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (7, 6))])),
        imp(0)
    );
    // deciding super tie-break: missing points, full set, wrong winner
    let pre = vec![s(6, 4), s(3, 6)];
    let with = |last| {
        let mut v = pre.clone();
        v.push(last);
        Sets(v)
    };
    assert_eq!(
        validate_final(&sup, &with(s(1, 0))),
        Err(InvalidScore::MissingTieBreak { index: 2 })
    );
    assert_eq!(validate_final(&sup, &with(s(6, 4))), imp(2));
    assert_eq!(validate_final(&sup, &with(stb(1, 0, (7, 10)))), imp(2));
    assert_eq!(validate_final(&sup, &with(stb(1, 1, (10, 7)))), imp(2));
    // a super tie-break is not used when a team already won 2-0
    assert_eq!(
        validate_final(&sup, &Sets(vec![s(6, 4), s(6, 3), stb(1, 0, (10, 7))])),
        Err(InvalidScore::ExtraSets)
    );
}

#[test]
fn final_score_rejects_unreachable_tiebreaks() {
    let full = fmt(BestOf3, FullSet);
    let sup = fmt(BestOf3, SuperTieBreak);
    let imp = |index| Err(InvalidScore::ImpossibleSet { index });
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (9, 2))])),
        imp(0)
    );
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (12, 3))])),
        imp(0)
    );
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (8, 4))])),
        imp(0)
    );
    let pre = |last| Sets(vec![s(6, 4), s(3, 6), last]);
    assert_eq!(validate_final(&sup, &pre(stb(1, 0, (15, 3)))), imp(2));
    assert_eq!(validate_final(&sup, &pre(stb(1, 0, (11, 5)))), imp(2));
    assert_eq!(
        validate_final(&full, &Sets(vec![stb(7, 6, (12, 10)), s(6, 0)])),
        Ok(Some(Team::A))
    );
    assert_eq!(
        validate_final(&sup, &pre(stb(1, 0, (12, 10)))),
        Ok(Some(Team::A))
    );
}

#[test]
fn final_score_rejects_invalid_rules() {
    assert_eq!(
        validate_final(&fmt(One, SuperTieBreak), &Sets(vec![s(6, 4)])),
        Err(InvalidScore::InvalidRules(
            RulesError::SuperTieBreakNeedsMultipleSets
        ))
    );
}

fn play(format: &Format, points: &str) -> Score {
    let start = Start {
        first_server: Player {
            team: Team::A,
            index: 0,
        },
        other_first_server: 0,
    };
    let events: Vec<Event> = points
        .chars()
        .filter_map(|c| match c {
            'a' => Some(Event::Point(Team::A)),
            'b' => Some(Event::Point(Team::B)),
            't' => Some(Event::TimeUp),
            _ => None,
        })
        .collect();
    replay(format, &start, &events).unwrap()
}

#[test]
fn final_score_round_trips_through_validate_final() {
    // Timed: 1-1 at time-up, B wins the overtime game.
    let f = timed(false);
    let score = play(&f, "aaaa bbbb t bbbb");
    assert_eq!(score.games, [1, 2]);
    assert_eq!(score.regulation_games, [1, 1]);
    let typed = score.final_score().unwrap();
    assert_eq!(
        typed,
        Timed {
            a: 1,
            b: 1,
            overtime_winner: Some(Team::B)
        }
    );
    assert_eq!(validate_final(&f, &typed), Ok(Some(Team::B)));

    // Timed without overtime: regulation games are the games.
    let f = timed(true);
    let score = play(&f, "aaaa aaaa bbbb t");
    assert_eq!(score.regulation_games, score.games);
    let typed = score.final_score().unwrap();
    assert_eq!(
        typed,
        Timed {
            a: 2,
            b: 1,
            overtime_winner: None
        }
    );
    assert_eq!(validate_final(&f, &typed), Ok(Some(Team::A)));

    // Sets: 6-0 6-4.
    let f = fmt(BestOf3, FullSet);
    let points = format!("{}{}aaaaaaaa", "aaaa".repeat(6), "aaaabbbb".repeat(4));
    let score = play(&f, &points);
    assert_eq!(score.regulation_games, score.games);
    let typed = score.final_score().unwrap();
    assert_eq!(typed, Sets(vec![s(6, 0), s(6, 4)]));
    assert_eq!(validate_final(&f, &typed), Ok(Some(Team::A)));

    // Not finished: no final score.
    assert_eq!(play(&f, "aaaa").final_score(), None);
    assert_eq!(play(&timed(false), "aaaa bbbb t").final_score(), None);
}
