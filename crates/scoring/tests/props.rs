//! Property tests over the whole engine: replay, undo, rejected events and final-score consistency.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use padel_scoring::*;
use proptest::prelude::*;

fn team() -> impl Strategy<Value = Team> {
    prop_oneof![Just(Team::A), Just(Team::B)]
}

fn deuce() -> impl Strategy<Value = DeuceRule> {
    prop_oneof![
        Just(DeuceRule::Advantage),
        Just(DeuceRule::GoldenPoint),
        Just(DeuceRule::StarPoint),
    ]
}

/// Any `Sets` format that passes `validate()`.
fn sets_format() -> impl Strategy<Value = Format> {
    let sets = prop_oneof![Just(SetsFormat::One), Just(SetsFormat::BestOf3)];
    let deciding = prop_oneof![Just(DecidingSet::FullSet), Just(DecidingSet::SuperTieBreak)];
    (deuce(), sets, deciding)
        .prop_map(|(deuce, sets, deciding)| {
            Format::Sets(SetRules {
                deuce,
                sets,
                deciding,
            })
        })
        .prop_filter("format must validate", |f| f.validate().is_ok())
}

fn timed_format() -> impl Strategy<Value = Format> {
    (deuce(), any::<bool>()).prop_map(|(deuce, draws_allowed)| Format::Timed {
        deuce,
        draws_allowed,
    })
}

fn format() -> impl Strategy<Value = Format> {
    prop_oneof![sets_format(), timed_format()]
}

fn start() -> impl Strategy<Value = Start> {
    (team(), 0u8..=1).prop_map(|(team, index)| Start {
        first_server: Player { team, index },
    })
}

const MAX_EVENTS: usize = 399;

/// Runs of 1-8 points for one team, so games and sets actually get decided,
/// with a `TimeUp` about once every 40 events (one chunk in ten, runs average 4.5 points).
fn events() -> impl Strategy<Value = Vec<Event>> {
    let run = (team(), 1usize..=8).prop_map(|(t, n)| vec![Event::Point(t); n]);
    let chunk = prop_oneof![9 => run, 1 => Just(vec![Event::TimeUp])];
    prop::collection::vec(chunk, 0..90).prop_map(flatten)
}

/// Points only, in runs that lean towards one team, so most sets matches finish.
fn finishing_points() -> impl Strategy<Value = Vec<Event>> {
    (team(), prop::collection::vec((0u8..3, 1usize..=8), 0..90)).prop_map(|(lead, runs)| {
        let chunks = runs
            .into_iter()
            .map(|(pick, n)| {
                // Two runs in three go to the leading team.
                let t = if pick < 2 { lead } else { lead.other() };
                vec![Event::Point(t); n]
            })
            .collect();
        flatten(chunks)
    })
}

fn flatten(chunks: Vec<Vec<Event>>) -> Vec<Event> {
    let mut events: Vec<Event> = chunks.into_iter().flatten().collect();
    events.truncate(MAX_EVENTS);
    events
}

/// The longest prefix of `events` that replays `Ok`.
fn valid_prefix(format: &Format, start: &Start, events: &[Event]) -> Vec<Event> {
    match replay(format, start, events) {
        Ok(_) => events.to_vec(),
        Err(e) => events[..e.index].to_vec(),
    }
}

/// The typed final score a scorer would enter for a finished match.
fn final_score_of(score: &Score) -> FinalScore {
    match score.status {
        Status::Finished { winner } if score.time_up => {
            let [mut a, mut b] = score.games;
            let overtime_winner = if score.overtime { winner } else { None };
            // `FinalScore::Timed` holds the regulation games: take the overtime game back off.
            match overtime_winner {
                Some(Team::A) => a -= 1,
                Some(Team::B) => b -= 1,
                None => {}
            }
            FinalScore::Timed {
                a,
                b,
                overtime_winner,
            }
        }
        _ => FinalScore::Sets(score.sets.clone()),
    }
}

proptest! {
    #[test]
    fn never_panics(format in format(), start in start(), events in events()) {
        let _ = replay(&format, &start, &events);
    }

    /// The server's undo is "drop the last event, then replay": it must give back exactly
    /// the score before that event.
    #[test]
    fn prefix_replay_equals_undo(format in format(), start in start(), events in events()) {
        let events = valid_prefix(&format, &start, &events);
        prop_assume!(!events.is_empty());
        let (last, prefix) = events.split_last().unwrap();
        let mut m = MatchState::new(format, start).unwrap();
        for e in prefix {
            m.apply(*e).unwrap();
        }
        let before = m.score();
        prop_assert_eq!(&before, &replay(&format, &start, prefix).unwrap());
        m.apply(*last).unwrap();
        prop_assert_eq!(m.score(), replay(&format, &start, &events).unwrap());
        // Undo the last event.
        prop_assert_eq!(replay(&format, &start, prefix).unwrap(), before);
    }

    #[test]
    fn rejected_event_leaves_state_unchanged(
        format in format(),
        start in start(),
        events in events(),
    ) {
        let mut m = MatchState::new(format, start).unwrap();
        for e in events {
            let before = m.clone();
            if m.apply(e).is_err() {
                prop_assert_eq!(&m, &before);
                prop_assert_eq!(m.score(), before.score());
            }
        }
    }

    #[test]
    fn finished_sets_match_validates(
        format in sets_format(),
        start in start(),
        events in finishing_points(),
    ) {
        let events = valid_prefix(&format, &start, &events);
        let score = replay(&format, &start, &events).unwrap();
        if let Status::Finished { winner } = score.status {
            prop_assert!(winner.is_some());
            prop_assert_eq!(validate_final(&format, &final_score_of(&score)), Ok(winner));
        }
    }

    #[test]
    fn finished_timed_match_validates(
        format in timed_format(),
        start in start(),
        events in events(),
    ) {
        let events = valid_prefix(&format, &start, &events);
        let score = replay(&format, &start, &events).unwrap();
        if let Status::Finished { winner } = score.status {
            prop_assert_eq!(validate_final(&format, &final_score_of(&score)), Ok(winner));
        }
    }

    #[test]
    fn games_only_grow(format in timed_format(), start in start(), events in events()) {
        let mut m = MatchState::new(format, start).unwrap();
        let mut total = 0u32;
        for e in events {
            let _ = m.apply(e);
            let [a, b] = m.score().games;
            let now = u32::from(a) + u32::from(b);
            prop_assert!(now >= total, "games went from {} to {}", total, now);
            total = now;
        }
    }
}
