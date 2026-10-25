use crate::state::set_won;
use crate::tiebreak::{SUPER_TIEBREAK_TO, TIEBREAK_TO, tiebreak_won};
use crate::{DecidingSet, Format, RulesError, SetRules, SetScore, SetsFormat, Team};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A final score typed in after the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalScore {
    /// The sets in order, shaped like the engine's `Score.sets`: a 7-6 set may omit its tie-break
    /// points, and a deciding super tie-break is `1-0` or `0-1` with the points in `tiebreak`.
    Sets(Vec<SetScore>),
    /// `a` and `b` are the regulation games, counted before any overtime game.
    /// `overtime_winner` is who won the overtime game; it is only valid when `a == b`
    /// and the format does not allow draws.
    Timed {
        a: u16,
        b: u16,
        overtime_winner: Option<Team>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidScore {
    #[error("invalid rules: {0}")]
    InvalidRules(RulesError),
    #[error("the score does not match the match format")]
    WrongFormat,
    #[error("set {index} is not a possible score")]
    ImpossibleSet { index: usize },
    #[error("set {index} needs its tie-break points")]
    MissingTieBreak { index: usize },
    #[error("the match is not finished")]
    MatchNotFinished,
    #[error("sets were played after the match was decided")]
    ExtraSets,
    #[error("a draw is not allowed in this format")]
    DrawNotAllowed,
    #[error("an overtime winner is only valid when the games are level and draws are not allowed")]
    OvertimeWinnerWhenNotLevel,
}

/// Check a typed final score against the format. Returns the winner, or `None` for a draw.
pub fn validate_final(format: &Format, score: &FinalScore) -> Result<Option<Team>, InvalidScore> {
    format.validate().map_err(InvalidScore::InvalidRules)?;
    match (format, score) {
        (Format::Sets(rules), FinalScore::Sets(sets)) => validate_sets(rules, sets).map(Some),
        (
            Format::Timed { draws_allowed, .. },
            FinalScore::Timed {
                a,
                b,
                overtime_winner,
            },
        ) => validate_timed(*draws_allowed, *a, *b, *overtime_winner),
        _ => Err(InvalidScore::WrongFormat),
    }
}

fn validate_timed(
    draws_allowed: bool,
    a: u16,
    b: u16,
    overtime_winner: Option<Team>,
) -> Result<Option<Team>, InvalidScore> {
    if a != b {
        return match overtime_winner {
            Some(_) => Err(InvalidScore::OvertimeWinnerWhenNotLevel),
            None if a > b => Ok(Some(Team::A)),
            None => Ok(Some(Team::B)),
        };
    }
    match (draws_allowed, overtime_winner) {
        (true, None) => Ok(None),
        (true, Some(_)) => Err(InvalidScore::OvertimeWinnerWhenNotLevel),
        (false, Some(w)) => Ok(Some(w)),
        (false, None) => Err(InvalidScore::DrawNotAllowed),
    }
}

fn validate_sets(rules: &SetRules, sets: &[SetScore]) -> Result<Team, InvalidScore> {
    let to_win = rules.sets.sets_to_win();
    let mut won = [0u8; 2];
    for (index, set) in sets.iter().enumerate() {
        if won.iter().any(|&w| w >= to_win) {
            return Err(InvalidScore::ExtraSets);
        }
        let deciding_super = rules.sets == SetsFormat::BestOf3
            && rules.deciding == DecidingSet::SuperTieBreak
            && won == [1, 1];
        let winner = if deciding_super {
            super_tiebreak_winner(index, set)?
        } else {
            regular_set_winner(index, set)?
        };
        let slot = match winner {
            Team::A => 0,
            Team::B => 1,
        };
        if let Some(w) = won.get_mut(slot) {
            *w = w.saturating_add(1);
        }
    }
    match won {
        [a, _] if a >= to_win => Ok(Team::A),
        [_, b] if b >= to_win => Ok(Team::B),
        _ => Err(InvalidScore::MatchNotFinished),
    }
}

fn regular_set_winner(index: usize, set: &SetScore) -> Result<Team, InvalidScore> {
    let impossible = InvalidScore::ImpossibleSet { index };
    let winner = set_won(set.a, set.b).ok_or(impossible)?;
    // `set_won` also accepts unreachable scores such as 8-6 or 7-3. Requiring that the set was not
    // already won one game earlier leaves only the reachable sets: 6-0 to 6-4, 7-5 and 7-6.
    let already_won = match winner {
        Team::A => set_won(set.a.saturating_sub(1), set.b),
        Team::B => set_won(set.a, set.b.saturating_sub(1)),
    };
    if already_won.is_some() {
        return Err(impossible);
    }
    if let Some((x, y)) = set.tiebreak {
        let is_tiebreak_set = set.a.max(set.b) == 7 && set.a.min(set.b) == 6;
        if !is_tiebreak_set || reachable_tiebreak_winner(x, y, TIEBREAK_TO) != Some(winner) {
            return Err(impossible);
        }
    }
    Ok(winner)
}

fn super_tiebreak_winner(index: usize, set: &SetScore) -> Result<Team, InvalidScore> {
    let impossible = InvalidScore::ImpossibleSet { index };
    let winner = match (set.a, set.b) {
        (1, 0) => Team::A,
        (0, 1) => Team::B,
        _ => return Err(impossible),
    };
    let (x, y) = set
        .tiebreak
        .ok_or(InvalidScore::MissingTieBreak { index })?;
    if reachable_tiebreak_winner(x, y, SUPER_TIEBREAK_TO) != Some(winner) {
        return Err(impossible);
    }
    Ok(winner)
}

/// The winner of a tie-break to `target` at `a`-`b`, only if play could actually have ended there:
/// it must not already have been won one point earlier (rules out 9-2, 15-3, ...).
fn reachable_tiebreak_winner(a: u16, b: u16, target: u8) -> Option<Team> {
    let winner = tiebreak_won(a, b, target)?;
    let earlier = match winner {
        Team::A => tiebreak_won(a.saturating_sub(1), b, target),
        Team::B => tiebreak_won(a, b.saturating_sub(1), target),
    };
    earlier.is_none().then_some(winner)
}
