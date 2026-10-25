use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Team {
    A,
    B,
}

impl Team {
    pub fn other(self) -> Team {
        match self {
            Team::A => Team::B,
            Team::B => Team::A,
        }
    }
}

/// A player on a team. `index` is 0 or 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Player {
    pub team: Team,
    pub index: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeuceRule {
    Advantage,
    GoldenPoint,
    StarPoint,
}

/// With `StarPoint`, the deciding point is played at the third deuce.
pub const STAR_POINT_DEUCE: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetsFormat {
    One,
    #[serde(rename = "best_of_3")]
    BestOf3,
}

impl SetsFormat {
    pub fn sets_to_win(self) -> u8 {
        match self {
            SetsFormat::One => 1,
            SetsFormat::BestOf3 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidingSet {
    FullSet,
    SuperTieBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetRules {
    pub deuce: DeuceRule,
    pub sets: SetsFormat,
    pub deciding: DecidingSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Sets(SetRules),
    Timed {
        deuce: DeuceRule,
        draws_allowed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulesError {
    /// A one-set match has no deciding set to replace.
    #[error("a super tie-break needs a match of more than one set")]
    SuperTieBreakNeedsMultipleSets,
    #[error("player index must be 0 or 1")]
    InvalidPlayerIndex,
}

impl Format {
    pub fn validate(&self) -> Result<(), RulesError> {
        match self {
            Format::Sets(SetRules {
                sets: SetsFormat::One,
                deciding: DecidingSet::SuperTieBreak,
                ..
            }) => Err(RulesError::SuperTieBreakNeedsMultipleSets),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Start {
    pub first_server: Player,
    /// The index (0 or 1) of the player who serves first for the team that receives first.
    pub other_first_server: u8,
}

impl Start {
    pub fn validate(&self) -> Result<(), RulesError> {
        if self.first_server.index > 1 || self.other_first_server > 1 {
            return Err(RulesError::InvalidPlayerIndex);
        }
        Ok(())
    }
}

/// One thing that happened in a match. The engine only ever sees points and time-up: the server
/// resolves an undo by removing the target event from its log and replaying the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Point(Team),
    TimeUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    #[error("the match is already over")]
    MatchOver,
    #[error("time-up is only valid in a timed match")]
    NotTimed,
    #[error("time-up was already called")]
    AlreadyTimeUp,
    #[error("invalid rules: {0}")]
    InvalidRules(RulesError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("event {index}: {kind}")]
pub struct ReplayError {
    pub index: usize,
    pub kind: ErrorKind,
}
