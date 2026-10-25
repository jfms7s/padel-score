#![allow(dead_code)]

use padel_scoring::*;

/// `'a'` is `Point(A)`, `'b'` is `Point(B)`, `'t'` is `TimeUp`; anything else (spaces) is ignored.
pub fn ev(s: &str) -> Vec<Event> {
    s.chars()
        .filter_map(|c| match c {
            'a' => Some(Event::Point(Team::A)),
            'b' => Some(Event::Point(Team::B)),
            't' => Some(Event::TimeUp),
            _ => None,
        })
        .collect()
}

pub fn sets(deuce: DeuceRule, sets: SetsFormat, deciding: DecidingSet) -> Format {
    Format::Sets(SetRules {
        deuce,
        sets,
        deciding,
    })
}

pub fn timed(deuce: DeuceRule, draws_allowed: bool) -> Format {
    Format::Timed {
        deuce,
        draws_allowed,
    }
}

pub fn start_a() -> Start {
    Start {
        first_server: Player {
            team: Team::A,
            index: 0,
        },
    }
}
