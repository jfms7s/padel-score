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
        other_first_server: 0,
    }
}

/// Golden point, best of 3, full deciding set.
pub fn bo3() -> Format {
    sets(
        DeuceRule::GoldenPoint,
        SetsFormat::BestOf3,
        DecidingSet::FullSet,
    )
}

/// Points for a run of games reaching `n_a`-`n_b`, each game won to love ("aaaa"/"bbbb").
/// Games alternate A, B, A, B, ... and the leader's surplus games come last,
/// so the set can't end before the requested score.
pub fn games(n_a: u8, n_b: u8) -> String {
    let level = n_a.min(n_b);
    let mut s = "aaaabbbb".repeat(usize::from(level));
    let (leader, surplus) = if n_a > n_b {
        ("aaaa", n_a - n_b)
    } else {
        ("bbbb", n_b - n_a)
    };
    s.push_str(&leader.repeat(usize::from(surplus)));
    s
}
