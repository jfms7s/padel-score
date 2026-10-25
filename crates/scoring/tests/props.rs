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
    (team(), 0u8..=1, 0u8..=1).prop_map(|(team, index, other_first_server)| Start {
        first_server: Player { team, index },
        other_first_server,
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

/// A sets format with points for it: a third lean towards one team, the rest are played set by set
/// with independent set winners (deciding sets, super tie-breaks and 7-6 sets).
fn sets_case() -> impl Strategy<Value = (Format, Vec<Event>)> {
    sets_format().prop_flat_map(|format| {
        let planned = set_plan().prop_map(move |plan| play_plan(&format, &plan));
        (
            Just(format),
            prop_oneof![1 => finishing_points(), 2 => planned],
        )
    })
}

/// How one set is to be played in a `SetPlan`.
#[derive(Debug, Clone)]
struct PlannedSet {
    winner: Team,
    /// Games the loser takes: 0-4 for 6-x, 5 for 7-5, 6 for 7-6 with a tie-break.
    loser_games: u8,
    /// Points the loser takes in each game before the winner's four (0-2, so four always win).
    game_noise: Vec<u8>,
    /// Points the loser takes in a tie-break before the winner's run (0-5, or 0-8 in a super tie-break).
    tiebreak_noise: u8,
}

/// Three sets with independent winners, so best-of-3 matches often reach 1-1
/// (and a deciding super tie-break), and some sets reach 6-6 and a tie-break.
fn set_plan() -> impl Strategy<Value = Vec<PlannedSet>> {
    let set = (team(), 0u8..=6, prop::collection::vec(0u8..=2, 13), 0u8..=8).prop_map(
        |(winner, loser_games, game_noise, tiebreak_noise)| PlannedSet {
            winner,
            loser_games,
            game_noise,
            tiebreak_noise,
        },
    );
    prop::collection::vec(set, 3)
}

/// The points that play out `plan` under `format`. A deciding set under `SuperTieBreak`
/// is played as a super tie-break; anything after the match is over is cut off by `valid_prefix`.
fn play_plan(format: &Format, plan: &[PlannedSet]) -> Vec<Event> {
    let super_deciding = matches!(
        format,
        Format::Sets(SetRules {
            sets: SetsFormat::BestOf3,
            deciding: DecidingSet::SuperTieBreak,
            ..
        })
    );
    let mut events = Vec::new();
    let mut won = [0u8; 2];
    let tiebreak = |events: &mut Vec<Event>, w: Team, noise: u8, to: usize| {
        events.extend(vec![Event::Point(w.other()); usize::from(noise)]);
        events.extend(vec![Event::Point(w); to]);
    };
    for set in plan {
        let w = set.winner;
        if super_deciding && won == [1, 1] {
            tiebreak(&mut events, w, set.tiebreak_noise, 10);
        } else {
            // Alternate games up to the loser's total, then the winner's remaining games.
            let level = set.loser_games.min(5);
            let mut order = Vec::new();
            for _ in 0..level {
                order.extend([w, w.other()]);
            }
            match set.loser_games {
                6 => order.extend([w.other(), w]),
                5 => order.push(w),
                _ => {}
            }
            let winner_left = match set.loser_games {
                6 => 0,
                5 => 1,
                l => 6 - l,
            };
            order.extend(vec![w; usize::from(winner_left)]);
            for (g, noise) in order.iter().zip(set.game_noise.iter().cycle()) {
                events.extend(vec![Event::Point(g.other()); usize::from(*noise)]);
                events.extend(vec![Event::Point(*g); 4]);
            }
            if set.loser_games == 6 {
                // 6-6: a tie-break to 7 with the loser on at most 5.
                tiebreak(&mut events, w, set.tiebreak_noise.min(5), 7);
            }
        }
        won[usize::from(w == Team::B)] += 1;
    }
    events
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
    fn finished_sets_match_validates((format, events) in sets_case(), start in start()) {
        let events = valid_prefix(&format, &start, &events);
        let score = replay(&format, &start, &events).unwrap();
        if let Status::Finished { winner } = score.status {
            prop_assert!(winner.is_some());
            let typed = score.final_score().unwrap();
            prop_assert!(matches!(typed, FinalScore::Sets(_)));
            prop_assert_eq!(validate_final(&format, &typed), Ok(winner));
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
            let typed = score.final_score().unwrap();
            prop_assert!(matches!(typed, FinalScore::Timed { .. }), "not a timed final score");
            prop_assert_eq!(validate_final(&format, &typed), Ok(winner));
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
