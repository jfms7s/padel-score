use crate::{
    DecidingSet, ErrorKind, Event, Format, Game, GameScore, Player, RulesError, SUPER_TIEBREAK_TO,
    ServeSide, SetRules, Start, TIEBREAK_TO, Team, TieBreak,
};
use serde::{Deserialize, Serialize};

/// Games needed to win a regular set (with a 2-game lead, or 7-6 via a tie-break).
const SET_GAMES: u8 = 6;

/// Who has won a regular set at `games_a`-`games_b`, if anyone:
/// at least 6 games with a lead of at least 2, or 7-6.
pub(crate) fn set_won(games_a: u8, games_b: u8) -> Option<Team> {
    let won = |own: u8, other: u8| {
        own >= SET_GAMES
            && (own.saturating_sub(other) >= 2 || (own == SET_GAMES + 1 && other == SET_GAMES))
    };
    if won(games_a, games_b) {
        Some(Team::A)
    } else if won(games_b, games_a) {
        Some(Team::B)
    } else {
        None
    }
}

/// A set's score. For a super tie-break set, `a`/`b` are 1-0 or 0-1 and `tiebreak` holds the points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetScore {
    pub a: u8,
    pub b: u8,
    pub tiebreak: Option<(u16, u16)>,
}

/// What is being played right now. `None` means the match is over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Current {
    Game(GameScore),
    TieBreak {
        a: u16,
        b: u16,
        super_tiebreak: bool,
    },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    InProgress,
    /// `winner` is `None` only for a timed draw.
    Finished {
        winner: Option<Team>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Score {
    /// The finished sets, then the one in progress.
    pub sets: Vec<SetScore>,
    pub sets_won: [u8; 2],
    /// Games in the current set; for a timed match, the whole match.
    pub games: [u16; 2],
    pub current: Current,
    pub server: Player,
    pub side: ServeSide,
    pub change_ends: bool,
    pub overtime: bool,
    pub time_up: bool,
    pub status: Status,
}

/// A match in progress. Feed it events with `apply`; read it with `score`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchState {
    start: Start,
    mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Sets(SetsMatch),
    // TODO(Task 6): timed-format state. Until then a timed match accepts events and ignores them.
    Timed,
}

impl MatchState {
    pub fn new(format: Format, start: Start) -> Result<MatchState, RulesError> {
        format.validate()?;
        start.validate()?;
        let mode = match format {
            Format::Sets(rules) => Mode::Sets(SetsMatch::new(rules)),
            Format::Timed { .. } => Mode::Timed,
        };
        Ok(MatchState { start, mode })
    }

    /// Apply one event. On `Err` the state is unchanged.
    pub fn apply(&mut self, e: Event) -> Result<(), ErrorKind> {
        match &mut self.mode {
            Mode::Sets(m) => m.apply(e),
            // TODO(Task 6): timed-format events.
            Mode::Timed => Ok(()),
        }
    }

    pub fn score(&self) -> Score {
        match &self.mode {
            Mode::Sets(m) => m.score(self.start),
            // TODO(Task 6): timed-format score.
            Mode::Timed => Score::blank(self.start),
        }
    }
}

impl Score {
    /// A score with nothing played. Serving is fixed at the first server until Task 5.
    fn blank(start: Start) -> Score {
        Score {
            sets: Vec::new(),
            sets_won: [0, 0],
            games: [0, 0],
            current: Current::None,
            // TODO(Task 5): serving and side.
            server: start.first_server,
            side: ServeSide::Right,
            change_ends: false,
            overtime: false,
            time_up: false,
            status: Status::InProgress,
        }
    }
}

/// What is being played in a sets-format match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Unit {
    Game(Game),
    TieBreak {
        tb: TieBreak,
        super_tiebreak: bool,
    },
    /// The match is over.
    Over,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SetsMatch {
    rules: SetRules,
    finished: Vec<SetScore>,
    sets_won: [u8; 2],
    /// Games in the current set (the final set's once the match is over).
    games: [u8; 2],
    unit: Unit,
    winner: Option<Team>,
}

impl SetsMatch {
    fn new(rules: SetRules) -> SetsMatch {
        let mut m = SetsMatch {
            rules,
            finished: Vec::new(),
            sets_won: [0, 0],
            games: [0, 0],
            unit: Unit::Over,
            winner: None,
        };
        m.unit = m.first_unit_of_set();
        m
    }

    fn apply(&mut self, e: Event) -> Result<(), ErrorKind> {
        let team = match e {
            Event::TimeUp => return Err(ErrorKind::NotTimed),
            Event::Point(team) => team,
        };
        match &mut self.unit {
            Unit::Over => return Err(ErrorKind::MatchOver),
            Unit::Game(game) => {
                if let Some(w) = game.point(team) {
                    self.game_won(w, None);
                }
            }
            Unit::TieBreak { tb, super_tiebreak } => {
                if let Some(w) = tb.point(team) {
                    let points = Some(tb.points());
                    if *super_tiebreak {
                        let (a, b) = match w {
                            Team::A => (1, 0),
                            Team::B => (0, 1),
                        };
                        self.set_won(
                            w,
                            SetScore {
                                a,
                                b,
                                tiebreak: points,
                            },
                        );
                    } else {
                        self.game_won(w, points);
                    }
                }
            }
        }
        Ok(())
    }

    /// `w` won a game (or a regular tie-break, which counts as the set's 13th game).
    fn game_won(&mut self, w: Team, tiebreak: Option<(u16, u16)>) {
        let g = slot(&mut self.games, w);
        *g = g.saturating_add(1);
        let [a, b] = self.games;
        if let Some(set_winner) = set_won(a, b) {
            self.set_won(set_winner, SetScore { a, b, tiebreak });
        } else if a == SET_GAMES && b == SET_GAMES {
            self.unit = Unit::TieBreak {
                tb: TieBreak::new(TIEBREAK_TO),
                super_tiebreak: false,
            };
        } else {
            self.unit = Unit::Game(Game::new(self.rules.deuce));
        }
    }

    fn set_won(&mut self, w: Team, set: SetScore) {
        self.games = [set.a, set.b];
        self.finished.push(set);
        let s = slot(&mut self.sets_won, w);
        *s = s.saturating_add(1);
        if *s >= self.rules.sets.sets_to_win() {
            self.winner = Some(w);
            self.unit = Unit::Over;
        } else {
            self.games = [0, 0];
            self.unit = self.first_unit_of_set();
        }
    }

    /// A game, or a super tie-break when this is the deciding set and the rules call for one.
    fn first_unit_of_set(&self) -> Unit {
        let one_short = self.rules.sets.sets_to_win().saturating_sub(1);
        let deciding = self.sets_won == [one_short, one_short];
        if deciding && self.rules.deciding == DecidingSet::SuperTieBreak {
            Unit::TieBreak {
                tb: TieBreak::new(SUPER_TIEBREAK_TO),
                super_tiebreak: true,
            }
        } else {
            Unit::Game(Game::new(self.rules.deuce))
        }
    }

    fn score(&self, start: Start) -> Score {
        let [a, b] = self.games;
        let mut sets = self.finished.clone();
        let current = match &self.unit {
            Unit::Over => Current::None,
            Unit::Game(game) => {
                sets.push(SetScore {
                    a,
                    b,
                    tiebreak: None,
                });
                Current::Game(game.score())
            }
            Unit::TieBreak { tb, super_tiebreak } => {
                let (pa, pb) = tb.points();
                sets.push(SetScore {
                    a,
                    b,
                    tiebreak: Some((pa, pb)),
                });
                Current::TieBreak {
                    a: pa,
                    b: pb,
                    super_tiebreak: *super_tiebreak,
                }
            }
        };
        let status = match self.winner {
            Some(winner) => Status::Finished {
                winner: Some(winner),
            },
            None => Status::InProgress,
        };
        Score {
            sets,
            sets_won: self.sets_won,
            games: [u16::from(a), u16::from(b)],
            current,
            status,
            ..Score::blank(start)
        }
    }
}

fn slot<T>(pair: &mut [T; 2], team: Team) -> &mut T {
    let [a, b] = pair;
    match team {
        Team::A => a,
        Team::B => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_won_rule() {
        assert_eq!(set_won(6, 4), Some(Team::A));
        assert_eq!(set_won(4, 6), Some(Team::B));
        assert_eq!(set_won(6, 5), None);
        assert_eq!(set_won(7, 5), Some(Team::A));
        assert_eq!(set_won(6, 6), None);
        assert_eq!(set_won(7, 6), Some(Team::A));
        assert_eq!(set_won(6, 7), Some(Team::B));
        assert_eq!(set_won(5, 0), None);
    }
}
