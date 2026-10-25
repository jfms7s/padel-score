use crate::Team;
use serde::{Deserialize, Serialize};

/// Points needed to win a regular tie-break.
pub const TIEBREAK_TO: u8 = 7;
/// Points needed to win a super tie-break.
pub const SUPER_TIEBREAK_TO: u8 = 10;

/// A tie-break (first to `target`, win by 2). Use `SUPER_TIEBREAK_TO` for a super tie-break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TieBreak {
    target: u8,
    a: u16,
    b: u16,
    won_by: Option<Team>,
}

impl TieBreak {
    pub fn new(target: u8) -> TieBreak {
        TieBreak {
            target,
            a: 0,
            b: 0,
            won_by: None,
        }
    }

    /// Award a point; returns the winner if this point ends the tie-break.
    ///
    /// The result is latched: once a winner has been returned, later calls return
    /// that same winner and change nothing (the score and `points_played` stay fixed).
    pub fn point(&mut self, team: Team) -> Option<Team> {
        if self.won_by.is_some() {
            return self.won_by;
        }
        match team {
            Team::A => self.a = self.a.saturating_add(1),
            Team::B => self.b = self.b.saturating_add(1),
        }
        self.won_by = tiebreak_won(self.a, self.b, self.target);
        self.won_by
    }

    pub fn points(&self) -> (u16, u16) {
        (self.a, self.b)
    }

    pub fn points_played(&self) -> u16 {
        self.a.saturating_add(self.b)
    }
}

/// Who has won a tie-break to `target` at the score `a`-`b`, if anyone:
/// at least `target` points and a lead of at least 2.
pub(crate) fn tiebreak_won(a: u16, b: u16, target: u8) -> Option<Team> {
    let target = u16::from(target);
    if a >= target && a.saturating_sub(b) >= 2 {
        Some(Team::A)
    } else if b >= target && b.saturating_sub(a) >= 2 {
        Some(Team::B)
    } else {
        None
    }
}
