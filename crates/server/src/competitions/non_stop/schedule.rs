//! The non-stop schedule: a round robin by the circle method, split into waves of at most
//! `courts` matches. Pure: no I/O, no clock, never panics (bad input gives an empty plan).

/// One match of the schedule. `a` and `b` index the team list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pairing {
    /// 1-based across legs.
    pub round: u32,
    /// 0-based leg.
    pub leg: u32,
    /// 1-based within the round.
    pub index: u32,
    pub a: usize,
    pub b: usize,
}

/// One clock run: one wave of one round, with each match's court index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPlan {
    pub slot: u32,
    pub round: u32,
    pub wave: u32,
    pub matches: Vec<(Pairing, usize)>,
}

/// `legs x (n - 1)` rounds of `n / 2` pairings (circle method). Team 0 stays put while the
/// others rotate; team 0 alternates sides round by round, and every odd leg swaps all sides.
/// Anything but an even `n >= 2` with `legs >= 1` gives an empty schedule.
pub fn round_robin(n: usize, legs: u32) -> Vec<Vec<Pairing>> {
    if n < 2 || !n.is_multiple_of(2) || legs == 0 {
        return Vec::new();
    }
    let half = n / 2;
    let mut rounds = Vec::new();
    let mut round_no: u32 = 0;
    for leg in 0..legs {
        for r in 0..n - 1 {
            let mut rest: Vec<usize> = (1..n).collect();
            rest.rotate_right(r);
            let order: Vec<usize> = std::iter::once(0).chain(rest).collect();
            let (front, back) = order.split_at(half);
            round_no = round_no.saturating_add(1);
            let pairings = front
                .iter()
                .zip(back.iter().rev())
                .enumerate()
                .map(|(i, (&x, &y))| {
                    let flip_first = i == 0 && r % 2 == 1;
                    let flip_leg = leg % 2 == 1;
                    let (a, b) = if flip_first != flip_leg {
                        (y, x)
                    } else {
                        (x, y)
                    };
                    Pairing {
                        round: round_no,
                        leg,
                        index: u32::try_from(i + 1).unwrap_or(u32::MAX),
                        a,
                        b,
                    }
                })
                .collect();
            rounds.push(pairings);
        }
    }
    rounds
}

/// Splits each round into waves of at most `courts` matches, in pairing order: match `k`
/// (1-based) is wave `(k-1)/courts + 1` on court `(k-1) % courts`. Slots are numbered in
/// play order from 1. `courts == 0` gives an empty plan.
pub fn plan_slots(rounds: &[Vec<Pairing>], courts: usize) -> Vec<SlotPlan> {
    if courts == 0 {
        return Vec::new();
    }
    let mut slots = Vec::new();
    let mut slot_no: u32 = 0;
    for round in rounds {
        for (w, wave) in round.chunks(courts).enumerate() {
            let Some(first) = wave.first() else { continue };
            slot_no = slot_no.saturating_add(1);
            slots.push(SlotPlan {
                slot: slot_no,
                round: first.round,
                wave: u32::try_from(w + 1).unwrap_or(u32::MAX),
                matches: wave.iter().copied().zip(0..).collect(),
            });
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn evens() -> impl Iterator<Item = usize> {
        (2..=16).step_by(2)
    }

    #[test]
    fn every_pair_meets_legs_times() {
        for n in evens() {
            for legs in 1..=3u32 {
                let mut seen: HashMap<(usize, usize), u32> = HashMap::new();
                for p in round_robin(n, legs).iter().flatten() {
                    *seen.entry((p.a.min(p.b), p.a.max(p.b))).or_default() += 1;
                }
                assert_eq!(seen.len(), n * (n - 1) / 2, "n={n} legs={legs}");
                assert!(seen.values().all(|&c| c == legs), "n={n} legs={legs}");
            }
        }
    }

    #[test]
    fn no_team_twice_in_a_round() {
        for n in evens() {
            for legs in 1..=3u32 {
                let rounds = round_robin(n, legs);
                assert_eq!(rounds.len(), legs as usize * (n - 1));
                for (i, round) in rounds.iter().enumerate() {
                    assert_eq!(round.len(), n / 2);
                    let mut teams: Vec<usize> = round.iter().flat_map(|p| [p.a, p.b]).collect();
                    teams.sort_unstable();
                    assert_eq!(teams, (0..n).collect::<Vec<_>>(), "n={n} round={i}");
                    for (k, p) in round.iter().enumerate() {
                        assert_eq!(p.round as usize, i + 1);
                        assert_eq!(p.index as usize, k + 1);
                        assert_eq!(p.leg as usize, i / (n - 1));
                    }
                }
            }
        }
    }

    #[test]
    fn second_leg_swaps_sides() {
        let rounds = round_robin(4, 2);
        assert_eq!(rounds.len(), 6);
        let (first_leg, second_leg) = rounds.split_at(3);
        for (first, second) in first_leg.iter().zip(second_leg) {
            for (x, y) in first.iter().zip(second) {
                assert_eq!((x.a, x.b), (y.b, y.a));
            }
        }
    }

    #[test]
    fn eight_teams_two_courts_is_fourteen_slots() {
        let slots = plan_slots(&round_robin(8, 1), 2);
        assert_eq!(slots.len(), 14);
        for (i, s) in slots.iter().enumerate() {
            assert_eq!(s.slot as usize, i + 1);
            assert_eq!(s.round as usize, i / 2 + 1);
            assert_eq!(s.wave as usize, i % 2 + 1);
            let courts: Vec<usize> = s.matches.iter().map(|(_, c)| *c).collect();
            assert_eq!(courts, vec![0, 1]);
        }
    }

    #[test]
    fn more_courts_than_matches() {
        let slots = plan_slots(&round_robin(4, 1), 5);
        assert_eq!(slots.len(), 3);
        for s in &slots {
            assert_eq!(s.wave, 1);
            let courts: Vec<usize> = s.matches.iter().map(|(_, c)| *c).collect();
            assert_eq!(courts, vec![0, 1]);
        }
    }

    #[test]
    fn odd_or_empty_input_is_empty() {
        assert!(round_robin(5, 1).is_empty());
        assert!(round_robin(0, 1).is_empty());
        assert!(round_robin(4, 0).is_empty());
        assert!(plan_slots(&round_robin(4, 1), 0).is_empty());
    }
}
