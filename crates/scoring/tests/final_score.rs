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
