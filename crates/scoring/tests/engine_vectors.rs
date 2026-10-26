//! Engine vectors for the watch's Kotlin port: match scripts with the engine's score after every point.
//! The committed file is the contract; regenerate it with
//! `UPDATE_ENGINE_VECTORS=1 cargo test -p padel-scoring --test engine_vectors`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

mod support;

use padel_scoring::*;
use serde_json::{Value, json};
use support::{ev, games, sets};

const MAX_FILE_BYTES: usize = 1_000_000;
const MAX_EVENTS: usize = 2000;

const DEUCE_RULES: [(DeuceRule, &str); 3] = [
    (DeuceRule::Advantage, "advantage"),
    (DeuceRule::GoldenPoint, "golden"),
    (DeuceRule::StarPoint, "star"),
];
const FORMATS: [(SetsFormat, DecidingSet, &str); 3] = [
    (SetsFormat::One, DecidingSet::FullSet, "one"),
    (SetsFormat::BestOf3, DecidingSet::FullSet, "bo3"),
    (SetsFormat::BestOf3, DecidingSet::SuperTieBreak, "bo3-super"),
];

/// xorshift64*, so the vectors are reproducible without `rand`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// True with probability `num / den`.
    fn chance(&mut self, num: u64, den: u64) -> bool {
        (self.next() >> 11) % den < num
    }
}

/// The 8 combinations of first-server team x index x other team's first server, by `n`.
fn start_for(n: usize) -> Start {
    Start {
        first_server: Player {
            team: if n & 1 == 0 { Team::A } else { Team::B },
            index: u8::from(n & 2 != 0),
        },
        other_first_server: u8::from(n & 4 != 0),
    }
}

fn vector(name: &str, rules: SetRules, start: Start, events: &str) -> Value {
    let mut state = MatchState::new(Format::Sets(rules), start).unwrap();
    let steps: Vec<Value> = ev(events)
        .into_iter()
        .map(|e| match state.apply(e) {
            Ok(()) => serde_json::to_value(state.score()).unwrap(),
            Err(kind) => json!({ "error": kind }),
        })
        .collect();
    json!({
        "name": name,
        "rules": rules,
        "start": start,
        "events": events,
        "steps": steps,
    })
}

/// Random points until the match is finished, then one more (refused).
fn random_events(rules: SetRules, start: Start, seed: u64, num: u64, den: u64) -> String {
    let mut rng = Rng(seed);
    let mut state = MatchState::new(Format::Sets(rules), start).unwrap();
    let mut events = String::new();
    while !matches!(state.score().status, Status::Finished { .. }) && events.len() < MAX_EVENTS {
        let (c, team) = if rng.chance(num, den) {
            ('a', Team::A)
        } else {
            ('b', Team::B)
        };
        state.apply(Event::Point(team)).unwrap();
        events.push(c);
    }
    events.push('a');
    events
}

fn random_vectors(with_lopsided_one_set: bool) -> Vec<Value> {
    let mut out = Vec::new();
    let mut n = 0;
    for (deuce, deuce_name) in DEUCE_RULES {
        for (sets_format, deciding, format_name) in FORMATS {
            let rules = SetRules {
                deuce,
                sets: sets_format,
                deciding,
            };
            for (num, den, label) in [(1, 2, "even"), (13, 20, "lopsided")] {
                let skip =
                    label == "lopsided" && sets_format == SetsFormat::One && !with_lopsided_one_set;
                // Advance `n` regardless, so a dropped vector doesn't shift the other starts.
                let start = start_for(n);
                let seed = 0x9E37_79B9_7F4A_7C15_u64.wrapping_mul(n as u64 + 1);
                n += 1;
                if skip {
                    continue;
                }
                let events = random_events(rules, start, seed, num, den);
                let name = format!("{deuce_name}-{format_name}-{label}");
                out.push(vector(&name, rules, start, &events));
            }
        }
    }
    out
}

fn hand_written_vectors() -> Vec<Value> {
    let start = start_for(0);
    let rules = |deuce, deciding| SetRules {
        deuce,
        sets: SetsFormat::BestOf3,
        deciding,
    };
    let Format::Sets(golden) = sets(
        DeuceRule::GoldenPoint,
        SetsFormat::BestOf3,
        DecidingSet::FullSet,
    ) else {
        unreachable!()
    };
    let super_rules = SetRules {
        deciding: DecidingSet::SuperTieBreak,
        ..golden
    };

    let star = format!("aaabbb{}a", "ab".repeat(4));
    let advantage = format!("aaabbb{}aa", "ab".repeat(6));
    // 6-6 in games, then a tie-break to 7-5 through 5-5.
    let tiebreak = format!("{}{}aa", games(6, 6), "ab".repeat(5));
    // One set each, then a super tie-break to 10-8 through 8-8.
    let super_tiebreak = format!("{}{}{}aa", games(6, 0), games(0, 6), "ab".repeat(8));

    vec![
        vector(
            "star-point-long-deuce",
            rules(DeuceRule::StarPoint, DecidingSet::FullSet),
            start,
            &star,
        ),
        vector(
            "advantage-long-deuce",
            rules(DeuceRule::Advantage, DecidingSet::FullSet),
            start,
            &advantage,
        ),
        vector("tiebreak-7-5", golden, start, &tiebreak),
        vector("super-tiebreak-10-8", super_rules, start, &super_tiebreak),
    ]
}

fn render(vectors: &[Value]) -> String {
    let lines: Vec<String> = vectors
        .iter()
        .map(|v| serde_json::to_string(v).unwrap())
        .collect();
    format!("{{\"vectors\":[\n{}\n]}}\n", lines.join(",\n"))
}

fn build() -> String {
    let all = |with_lopsided_one_set| {
        let mut vectors = random_vectors(with_lopsided_one_set);
        vectors.extend(hand_written_vectors());
        render(&vectors)
    };
    let text = all(true);
    if text.len() < MAX_FILE_BYTES {
        text
    } else {
        all(false)
    }
}

#[test]
fn engine_vectors_match_the_committed_file() {
    let text = build();
    assert!(text.len() < MAX_FILE_BYTES, "{} bytes", text.len());

    let path = format!(
        "{}/tests/fixtures/engine-vectors.json",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("UPDATE_ENGINE_VECTORS").is_some() {
        std::fs::create_dir_all(format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))).unwrap();
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == text,
        "{path} is out of date; rerun with `UPDATE_ENGINE_VECTORS=1 cargo test -p padel-scoring --test engine_vectors`"
    );
}
