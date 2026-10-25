use crate::serve::{
    ends_change_after_game, ends_change_in_tiebreak, in_rotation, serve_side, tiebreak_server,
};
use crate::{
    DecidingSet, DeuceRule, ErrorKind, Event, FinalScore, Format, Game, GameScore, Player,
    RulesError, SUPER_TIEBREAK_TO, ServeSide, SetRules, Start, TIEBREAK_TO, Team, TieBreak,
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
    /// Once the match is over it shows the final set's games, so a super tie-break set shows 1-0 or 0-1.
    pub games: [u16; 2],
    /// The games before any overtime game. It equals `games` for a sets match and for a timed
    /// match with no overtime. Standings count regulation games only (spec §4.3 step 2).
    pub regulation_games: [u16; 2],
    pub current: Current,
    /// Who serves the next point.
    pub server: Player,
    /// Where the next serve goes from.
    pub side: ServeSide,
    /// The players change ends now: true only in the snapshot right after the event that caused it,
    /// and never once the match is over.
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
    Timed(TimedMatch),
}

impl MatchState {
    pub fn new(format: Format, start: Start) -> Result<MatchState, RulesError> {
        format.validate()?;
        start.validate()?;
        let mode = match format {
            Format::Sets(rules) => Mode::Sets(SetsMatch::new(rules)),
            Format::Timed {
                deuce,
                draws_allowed,
            } => Mode::Timed(TimedMatch::new(deuce, draws_allowed)),
        };
        Ok(MatchState { start, mode })
    }

    /// Apply one event. On `Err` the state is unchanged.
    pub fn apply(&mut self, e: Event) -> Result<(), ErrorKind> {
        match &mut self.mode {
            Mode::Sets(m) => m.apply(e),
            Mode::Timed(m) => m.apply(e),
        }
    }

    pub fn score(&self) -> Score {
        let mut score = match &self.mode {
            Mode::Sets(m) => m.score(self.start),
            Mode::Timed(m) => m.score(self.start),
        };
        // No change of ends after match point: the match is over.
        if score.status != Status::InProgress {
            score.change_ends = false;
        }
        score
    }
}

impl Score {
    /// The final score as it would be typed in after the match, for `validate_final`.
    /// `None` unless the match is finished.
    pub fn final_score(&self) -> Option<FinalScore> {
        let Status::Finished { winner } = self.status else {
            return None;
        };
        // Only a timed match has a time-up, and a timed match only finishes after one.
        if !self.time_up {
            return Some(FinalScore::Sets(self.sets.clone()));
        }
        let [a, b] = self.regulation_games;
        Some(FinalScore::Timed {
            a,
            b,
            overtime_winner: if self.overtime { winner } else { None },
        })
    }

    /// Defaults for the fields a mode doesn't set: no sets, no games, no time-up or overtime.
    /// Every mode sets `current`, `server`, `side`, `change_ends` and `status` itself.
    fn base(start: Start) -> Score {
        Score {
            sets: Vec::new(),
            sets_won: [0, 0],
            games: [0, 0],
            regulation_games: [0, 0],
            current: Current::None,
            server: start.first_server,
            side: ServeSide::Right,
            change_ends: false,
            overtime: false,
            time_up: false,
            status: Status::InProgress,
        }
    }
}

/// A timed (non-stop) match: games accumulate with no sets and no tie-breaks
/// until time is up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TimedMatch {
    deuce: DeuceRule,
    draws_allowed: bool,
    /// Games in the whole match.
    games: [u16; 2],
    /// Games before any overtime game.
    regulation_games: [u16; 2],
    unit: TimedUnit,
    /// Places along the service rotation (mod 4) of the current game's server.
    rotation: u16,
    /// Whether the last event made the players change ends.
    change_ends: bool,
    time_up: bool,
    overtime: bool,
}

/// What is being played in a timed match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TimedUnit {
    Game(Game),
    /// The match is over; `winner` is `None` for a draw.
    Over {
        winner: Option<Team>,
    },
}

impl TimedMatch {
    fn new(deuce: DeuceRule, draws_allowed: bool) -> TimedMatch {
        TimedMatch {
            deuce,
            draws_allowed,
            games: [0, 0],
            regulation_games: [0, 0],
            unit: TimedUnit::Game(Game::new(deuce)),
            rotation: 0,
            change_ends: false,
            time_up: false,
            overtime: false,
        }
    }

    fn apply(&mut self, e: Event) -> Result<(), ErrorKind> {
        let TimedUnit::Game(game) = &mut self.unit else {
            return Err(ErrorKind::MatchOver);
        };
        match e {
            Event::Point(team) => {
                self.change_ends = false;
                if let Some(w) = game.point(team) {
                    self.game_won(w);
                }
            }
            Event::TimeUp => {
                if self.time_up {
                    return Err(ErrorKind::AlreadyTimeUp);
                }
                self.time_up();
            }
        }
        Ok(())
    }

    /// `w` won the game in progress: count it, pass service on, and either
    /// end the match (an overtime game decides it) or start the next game.
    fn game_won(&mut self, w: Team) {
        let g = slot(&mut self.games, w);
        *g = g.saturating_add(1);
        let [a, b] = self.games;
        // The whole match is one "set" for the change-of-ends rule.
        self.rotation = self.rotation.wrapping_add(1) % 4;
        self.change_ends |= ends_change_after_game(a.saturating_add(b));
        if self.overtime {
            self.finish(Some(w));
        } else {
            self.regulation_games = self.games;
            self.unit = TimedUnit::Game(Game::new(self.deuce));
        }
    }

    /// Time is up: discard the game in progress, then the leader wins, or a level
    /// match is a draw or goes to a single overtime game. The discarded game doesn't
    /// move the rotation, so its server serves the overtime game.
    fn time_up(&mut self) {
        self.time_up = true;
        self.change_ends = false;
        let [a, b] = self.games;
        if a > b {
            self.finish(Some(Team::A));
        } else if b > a {
            self.finish(Some(Team::B));
        } else if self.draws_allowed {
            self.finish(None);
        } else {
            self.overtime = true;
            self.unit = TimedUnit::Game(Game::new(self.deuce));
        }
    }

    fn finish(&mut self, winner: Option<Team>) {
        self.unit = TimedUnit::Over { winner };
    }

    fn score(&self, start: Start) -> Score {
        let server = in_rotation(&start, self.rotation);
        let (current, side, status) = match &self.unit {
            TimedUnit::Game(game) => {
                let gs = game.score();
                let side = serve_side(game.points_played(), gs.deciding_point);
                (Current::Game(gs), side, Status::InProgress)
            }
            TimedUnit::Over { winner } => (
                Current::None,
                ServeSide::Right,
                Status::Finished { winner: *winner },
            ),
        };
        Score {
            games: self.games,
            regulation_games: self.regulation_games,
            current,
            server,
            side,
            change_ends: self.change_ends,
            overtime: self.overtime,
            time_up: self.time_up,
            status,
            ..Score::base(start)
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
    /// Places along the service rotation (mod 4) of the current game's server,
    /// or of the current tie-break's first server.
    rotation: u16,
    /// Whether the last event made the players change ends.
    change_ends: bool,
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
            rotation: 0,
            change_ends: false,
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
                self.change_ends = false;
                if let Some(w) = game.point(team) {
                    self.game_won(w, None);
                }
            }
            Unit::TieBreak { tb, super_tiebreak } => {
                let won = tb.point(team);
                let points = Some(tb.points());
                let super_tiebreak = *super_tiebreak;
                self.change_ends = ends_change_in_tiebreak(tb.points_played());
                if let Some(w) = won {
                    if super_tiebreak {
                        // A super tie-break is its set's only game.
                        self.game_finished(1);
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
        self.game_finished(u16::from(a).saturating_add(u16::from(b)));
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

    /// A game (or tie-break) ended with `games_in_set` games now played in its set:
    /// pass service to the next player in rotation and apply the change-of-ends rule.
    /// After a tie-break that player is the one after the tie-break's first server.
    fn game_finished(&mut self, games_in_set: u16) {
        self.rotation = self.rotation.wrapping_add(1) % 4;
        self.change_ends |= ends_change_after_game(games_in_set);
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
        let (server, side) = match &self.unit {
            Unit::Over => (in_rotation(&start, self.rotation), ServeSide::Right),
            Unit::Game(game) => (
                in_rotation(&start, self.rotation),
                serve_side(game.points_played(), game.score().deciding_point),
            ),
            Unit::TieBreak { tb, .. } => (
                tiebreak_server(&start, self.rotation, tb.points_played()),
                serve_side(tb.points_played(), false),
            ),
        };
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
            regulation_games: [u16::from(a), u16::from(b)],
            current,
            server,
            side,
            change_ends: self.change_ends,
            status,
            ..Score::base(start)
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
