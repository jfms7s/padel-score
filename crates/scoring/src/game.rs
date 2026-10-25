use crate::{DeuceRule, STAR_POINT_DEUCE, Team};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointCall {
    Love,
    Fifteen,
    Thirty,
    Forty,
    Advantage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameScore {
    pub a: PointCall,
    pub b: PointCall,
    /// How many times this game has reached 40-40.
    pub deuces: u8,
    /// True when the next point wins the game.
    pub deciding_point: bool,
}

/// A regular game. Raw points are tracked per team; calls and deuces are derived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    rule: DeuceRule,
    a: u16,
    b: u16,
    won_by: Option<Team>,
}

impl Game {
    pub fn new(rule: DeuceRule) -> Game {
        Game {
            rule,
            a: 0,
            b: 0,
            won_by: None,
        }
    }

    /// Award a point; returns the winner if this point ends the game.
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
        self.won_by = self.winner();
        self.won_by
    }

    pub fn points_played(&self) -> u16 {
        self.a.saturating_add(self.b)
    }

    pub fn score(&self) -> GameScore {
        GameScore {
            a: call(self.a, self.b),
            b: call(self.b, self.a),
            deuces: self.deuces(),
            deciding_point: self.a == self.b && self.a >= 3 && self.next_point_wins(),
        }
    }

    fn deuces(&self) -> u8 {
        let n = self.a.min(self.b).saturating_sub(2);
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    /// Lowest winning total at which a one-point lead already wins, if the rule has one.
    fn sudden_death_total(&self) -> Option<u16> {
        match self.rule {
            DeuceRule::Advantage => None,
            DeuceRule::GoldenPoint => Some(4),
            DeuceRule::StarPoint => Some(u16::from(STAR_POINT_DEUCE) + 3),
        }
    }

    /// For a level game at deuce: would the next point win outright?
    fn next_point_wins(&self) -> bool {
        self.sudden_death_total()
            .is_some_and(|total| self.a >= total.saturating_sub(1))
    }

    fn winner(&self) -> Option<Team> {
        let (leader, lead_pts, trail_pts) = if self.a >= self.b {
            (Team::A, self.a, self.b)
        } else {
            (Team::B, self.b, self.a)
        };
        let lead = lead_pts - trail_pts;
        let won = lead_pts >= 4
            && (lead >= 2
                || (lead >= 1 && self.sudden_death_total().is_some_and(|t| lead_pts >= t)));
        won.then_some(leader)
    }
}

fn call(own: u16, other: u16) -> PointCall {
    if own >= 3 && other >= 3 {
        return if own > other {
            PointCall::Advantage
        } else {
            PointCall::Forty
        };
    }
    match own {
        0 => PointCall::Love,
        1 => PointCall::Fifteen,
        2 => PointCall::Thirty,
        _ => PointCall::Forty,
    }
}
