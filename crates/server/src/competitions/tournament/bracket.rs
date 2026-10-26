//! The pure bracket core: the shape of a knockout draw of 4, 8, 16 or 32 teams, with and
//! without full placement, where each winner and loser goes, match labels, and a `Bracket`
//! model that turns "this match's result changed" into a `Plan` for the store to carry out.
//! No SQL and no I/O, so every rule here is unit- and property-tested on its own.
//!
//! A *draw* `(place_from, size)` covers places `place_from ..= place_from + size - 1`. Its match
//! `idx` meets slots `2 * idx` (side A) and `2 * idx + 1` (side B). Winners go to the draw
//! `(place_from, size / 2)`; losers to `(place_from + size / 2, size / 2)` if that draw is
//! played, else they share its places. In a size-2 draw the winner takes `place_from` and the
//! loser `place_from + 1`. A draw is played when `place_from == 1`, or full placement is on,
//! or it is the 3rd-place draw `(3, 2)`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::competitions::places::Place;

/// One match of the bracket: match `idx` of the draw `(place_from, size)`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Pos {
    pub place_from: u32,
    pub size: u32,
    pub idx: u32,
}

/// A match's slot: A meets B.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Side {
    A,
    B,
}

impl Side {
    /// The side a match `idx` of the draw before feeds: A from even matches, B from odd.
    pub fn of(idx: u32) -> Side {
        if idx.is_multiple_of(2) {
            Side::A
        } else {
            Side::B
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::A => Side::B,
            Side::B => Side::A,
        }
    }

    fn index(self) -> usize {
        match self {
            Side::A => 0,
            Side::B => 1,
        }
    }
}

/// Where a match's winner or loser goes: a slot of a later match, or a final place range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Slot(Pos, Side),
    Place { place: u32, place_to: u32 },
}

/// Where a slot's team comes from. Serialized as `{"winner_of": Pos}` / `{"loser_of": Pos}`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    WinnerOf(Pos),
    LoserOf(Pos),
}

/// The bracket's shape: the team count and whether every place is played for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    teams: u32,
    full_placement: bool,
}

impl Layout {
    /// `Some` only for 4, 8, 16 or 32 teams.
    pub fn new(teams: usize, full_placement: bool) -> Option<Layout> {
        let teams = match teams {
            4 => 4,
            8 => 8,
            16 => 16,
            32 => 32,
            _ => return None,
        };
        Some(Layout {
            teams,
            full_placement,
        })
    }

    /// How many teams the bracket is for.
    pub fn teams(&self) -> u32 {
        self.teams
    }

    /// Whether every place is played for.
    pub fn full_placement(&self) -> bool {
        self.full_placement
    }

    /// Whether the draw `(place_from, size)` exists in this bracket and is played.
    fn played(&self, place_from: u32, size: u32) -> bool {
        let shape_ok = size >= 2
            && size.is_power_of_two()
            && size <= self.teams
            && place_from >= 1
            && (place_from - 1).is_multiple_of(size)
            && place_from
                .checked_add(size - 1)
                .is_some_and(|last| last <= self.teams);
        shape_ok && (place_from == 1 || self.full_placement || (place_from == 3 && size == 2))
    }

    /// Whether `p` is a match of a played draw.
    fn contains(&self, p: Pos) -> bool {
        self.played(p.place_from, p.size) && p.idx < p.size / 2
    }

    /// Every match of every played draw, by `size` descending, then `place_from`, then `idx`.
    pub fn positions(&self) -> Vec<Pos> {
        let mut out = Vec::new();
        let mut size = self.teams;
        while size >= 2 {
            let mut place_from = 1;
            while place_from + size - 1 <= self.teams {
                if self.played(place_from, size) {
                    out.extend((0..size / 2).map(|idx| Pos {
                        place_from,
                        size,
                        idx,
                    }));
                }
                place_from += size;
            }
            size /= 2;
        }
        out
    }

    pub fn winner_target(&self, p: Pos) -> Target {
        if p.size <= 2 {
            return Target::Place {
                place: p.place_from,
                place_to: p.place_from,
            };
        }
        Target::Slot(
            Pos {
                place_from: p.place_from,
                size: p.size / 2,
                idx: p.idx / 2,
            },
            Side::of(p.idx),
        )
    }

    pub fn loser_target(&self, p: Pos) -> Target {
        let half = p.size / 2;
        let from = p.place_from.saturating_add(half.max(1));
        if p.size <= 2 {
            return Target::Place {
                place: from,
                place_to: from,
            };
        }
        if self.played(from, half) {
            return Target::Slot(
                Pos {
                    place_from: from,
                    size: half,
                    idx: p.idx / 2,
                },
                Side::of(p.idx),
            );
        }
        Target::Place {
            place: from,
            place_to: from.saturating_add(half - 1),
        }
    }

    /// The inverse of the targets: which match's winner or loser fills `side` of `p`. `None`
    /// for the first round of `(1, teams)` (and for a position outside this layout).
    pub fn source(&self, p: Pos, side: Side) -> Option<Source> {
        if !self.contains(p) || (p.place_from == 1 && p.size == self.teams) {
            return None;
        }
        let (q, m) = (p.place_from, p.size);
        let parent_size = m.checked_mul(2)?;
        let idx = p.idx.checked_mul(2)?.checked_add(side.index() as u32)?;
        if (q - 1).is_multiple_of(parent_size) && self.played(q, parent_size) {
            return Some(Source::WinnerOf(Pos {
                place_from: q,
                size: parent_size,
                idx,
            }));
        }
        let from = q.checked_sub(m)?;
        self.played(from, parent_size)
            .then_some(Source::LoserOf(Pos {
                place_from: from,
                size: parent_size,
                idx,
            }))
    }

    /// The match's name, at most 40 chars: "Quarter-final 1", "3rd place",
    /// "Placement semi 1 (5th–8th)".
    pub fn label(&self, p: Pos) -> String {
        let i = u64::from(p.idx) + 1;
        if p.place_from == 1 {
            return match p.size {
                2 => "Final".to_string(),
                4 => format!("Semi-final {i}"),
                8 => format!("Quarter-final {i}"),
                n => format!("Round of {n}, match {i}"),
            };
        }
        if p.size <= 2 {
            return format!("{} place", ordinal(p.place_from));
        }
        let round = match p.size {
            4 => "semi",
            8 => "QF",
            _ => "R16",
        };
        let to = p.place_from.saturating_add(p.size - 1);
        format!(
            "Placement {round} {i} ({}–{})",
            ordinal(p.place_from),
            ordinal(to)
        )
    }
}

/// English ordinal: 1st, 2nd, 3rd, 4th, 11th, 12th, 13th, 21st.
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// One bracket match as stored: its teams (`None` until known), its event match (`None` until
/// both teams are known), whether that match has started, its winner, and whether it is held
/// for the coordinator (its winner changed after a match it feeds started).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Node {
    pub pos: Pos,
    pub teams: [Option<String>; 2],
    pub match_id: Option<String>,
    pub started: bool,
    pub winner: Option<Side>,
    pub held: bool,
}

impl Node {
    fn team(&self, side: Side) -> Option<&String> {
        self.teams.get(side.index()).and_then(Option::as_ref)
    }

    /// The teams this node's result sends on: `(winner, loser)`, both `None` with no result.
    fn outcome(&self) -> (Option<String>, Option<String>) {
        match self.winner {
            Some(w) => (self.team(w).cloned(), self.team(w.other()).cloned()),
            None => (None, None),
        }
    }
}

/// A tournament's bracket: its layout and one node per position.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bracket {
    pub layout: Layout,
    pub nodes: Vec<Node>,
}

/// What the store does after a result changes, in this order: withdraw the matches (by match
/// id), set the slots, create the matches (in this order) and set the holds.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Plan {
    pub set: Vec<(Pos, Side, Option<String>)>,
    pub withdraw: Vec<String>,
    pub create: Vec<Pos>,
    pub held: Vec<(Pos, bool)>,
}

/// A slot whose team must change: `side` of `node` becomes `team`.
struct Change<'a> {
    node: &'a Node,
    side: Side,
    team: Option<String>,
}

impl Change<'_> {
    /// Whether the node has both teams once this change is made.
    fn completes(&self) -> bool {
        self.team.is_some() && self.node.team(self.side.other()).is_some()
    }
}

impl Bracket {
    pub fn node(&self, p: Pos) -> Option<&Node> {
        self.nodes.iter().find(|n| n.pos == p)
    }

    pub fn node_by_match(&self, match_id: &str) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|n| n.match_id.as_deref() == Some(match_id))
    }

    /// The slots `p`'s winner and loser should fill but don't yet.
    fn changes(&self, node: &Node) -> Vec<Change<'_>> {
        let (winner, loser) = node.outcome();
        let targets = [
            (self.layout.winner_target(node.pos), winner),
            (self.layout.loser_target(node.pos), loser),
        ];
        targets
            .into_iter()
            .filter_map(|(target, team)| match target {
                Target::Slot(t, side) => {
                    let tnode = self.node(t)?;
                    (tnode.team(side) != team.as_ref()).then_some(Change {
                        node: tnode,
                        side,
                        team,
                    })
                }
                Target::Place { .. } => None,
            })
            .collect()
    }

    /// The positions to create: the changed nodes that end up with both teams, the one
    /// deciding worse places first.
    fn creations(changes: &[Change<'_>]) -> Vec<Pos> {
        let mut create: Vec<Pos> = changes
            .iter()
            .filter(|c| c.completes())
            .map(|c| c.node.pos)
            .collect();
        create.sort_by_key(|p| std::cmp::Reverse(p.place_from));
        create
    }

    /// `p`'s result changed (a new result, a correction or an undo): bring its winner's and
    /// loser's slots in line. A target that has a started match is never touched: `p` is held
    /// instead. Never fails; an unknown `p` gives an empty plan.
    pub fn on_result(&self, p: Pos) -> Plan {
        let Some(node) = self.node(p) else {
            return Plan::default();
        };
        let changes = self.changes(node);
        let mut plan = Plan::default();
        if changes.iter().any(|c| c.node.started) {
            plan.held.push((p, true));
            return plan;
        }
        for c in &changes {
            plan.set.push((c.node.pos, c.side, c.team.clone()));
            plan.withdraw.extend(c.node.match_id.clone());
        }
        plan.create = Self::creations(&changes);
        if node.held {
            plan.held.push((p, false));
        }
        plan
    }

    /// The coordinator chose Replay on held `p`: like `on_result`, but a changed target's match
    /// is withdrawn even when started, and with it everything that match fed, recursively
    /// (each slot it fed is cleared, each match there withdrawn, each hold there cleared).
    pub fn replay(&self, p: Pos) -> Plan {
        let Some(node) = self.node(p) else {
            return Plan::default();
        };
        let changes = self.changes(node);
        let mut plan = Plan::default();
        let mut withdrawn = HashSet::new();
        for c in &changes {
            plan.set.push((c.node.pos, c.side, c.team.clone()));
            self.discard(c.node, &mut plan, &mut withdrawn);
        }
        plan.create = Self::creations(&changes);
        if node.held {
            plan.held.push((p, false));
        }
        plan
    }

    /// Withdraw `node`'s match (if any), clear its hold, and undo everything it fed.
    fn discard(&self, node: &Node, plan: &mut Plan, withdrawn: &mut HashSet<Pos>) {
        let Some(match_id) = &node.match_id else {
            return;
        };
        if !withdrawn.insert(node.pos) {
            return;
        }
        plan.withdraw.push(match_id.clone());
        if node.held {
            plan.held.push((node.pos, false));
        }
        for target in [
            self.layout.winner_target(node.pos),
            self.layout.loser_target(node.pos),
        ] {
            let Target::Slot(t, side) = target else {
                continue;
            };
            let Some(fed) = self.node(t) else {
                continue;
            };
            if fed.team(side).is_some() {
                plan.set.push((t, side, None));
            }
            self.discard(fed, plan, withdrawn);
        }
    }

    /// The nodes whose matches `replay(p)` withdraws, in `positions` order.
    pub fn replay_discards(&self, p: Pos) -> Vec<Pos> {
        let withdraw = self.replay(p).withdraw;
        self.layout
            .positions()
            .into_iter()
            .filter(|q| {
                self.node(*q)
                    .and_then(|n| n.match_id.as_ref())
                    .is_some_and(|id| withdraw.contains(id))
            })
            .collect()
    }

    /// The team `node` sends to its loser's place: the node's team that is not in its winner's
    /// target slot, when that slot holds one of its teams; else its result's loser. They differ
    /// after "Keep as played": the result was corrected but the team that played on stays in
    /// the later match, so the bracket as played decides who shares the place.
    pub fn placed_loser(&self, node: &Node) -> Option<String> {
        if let Target::Slot(t, side) = self.layout.winner_target(node.pos)
            && let Some(went_on) = self.node(t).and_then(|n| n.team(side))
        {
            for s in [Side::A, Side::B] {
                if node.team(s) == Some(went_on) {
                    return node.team(s.other()).cloned();
                }
            }
        }
        node.outcome().1
    }

    /// The final places once every match of every played draw has a winner and nothing is
    /// held; `None` before.
    pub fn places(&self) -> Option<Vec<Place>> {
        let mut out = Vec::new();
        for p in self.layout.positions() {
            let node = self.node(p)?;
            if node.held || node.winner.is_none() {
                return None;
            }
            let (winner, _) = node.outcome();
            for (target, team) in [
                (self.layout.winner_target(p), winner),
                (self.layout.loser_target(p), self.placed_loser(node)),
            ] {
                if let Target::Place { place, place_to } = target {
                    out.push(Place {
                        team_id: team?,
                        place,
                        place_to,
                    });
                }
            }
        }
        // The loop only visits played positions; a hold anywhere else still blocks the places.
        if self.nodes.iter().any(|n| n.held) {
            return None;
        }
        Some(out)
    }
}

/// Fisher–Yates with rejection sampling: given uniform `next_u32` values, every order of
/// `items` is equally likely. Values that would bias the pick are skipped and another is drawn.
pub fn shuffle<T>(items: &mut [T], mut next_u32: impl FnMut() -> u32) {
    for i in (1..items.len()).rev() {
        // More than u32::MAX items can't be shuffled with u32 draws; no bracket comes close.
        let Ok(bound) = u32::try_from(i + 1) else {
            continue;
        };
        // 2^32 mod bound: the values below it are the ones that would bias the pick.
        let reject_below = bound.wrapping_neg() % bound;
        let j = loop {
            let x = next_u32();
            if x >= reject_below {
                break x % bound;
            }
        };
        items.swap(i, j as usize);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use proptest::prelude::*;

    use super::*;
    use crate::competitions::places::validate_places;

    const SIZES: [usize; 4] = [4, 8, 16, 32];

    fn pos(place_from: u32, size: u32, idx: u32) -> Pos {
        Pos {
            place_from,
            size,
            idx,
        }
    }

    fn team(i: usize) -> String {
        format!("t{i}")
    }

    fn all_layouts() -> Vec<Layout> {
        SIZES
            .iter()
            .flat_map(|&n| [false, true].map(|full| Layout::new(n, full).unwrap()))
            .collect()
    }

    /// A bracket in memory, applying plans the way the store will: set slots, withdraw
    /// matches (their result goes with them), create matches, then set holds.
    struct Sim {
        b: Bracket,
        next_id: u32,
    }

    impl Sim {
        /// A started tournament: teams `t0..` in the first round's slots, its matches made.
        fn new(teams: usize, full_placement: bool) -> Sim {
            let layout = Layout::new(teams, full_placement).unwrap();
            let nodes = layout
                .positions()
                .into_iter()
                .map(|pos| Node {
                    pos,
                    teams: [None, None],
                    match_id: None,
                    started: false,
                    winner: None,
                    held: false,
                })
                .collect();
            let mut sim = Sim {
                b: Bracket { layout, nodes },
                next_id: 0,
            };
            let first: Vec<Pos> = sim
                .b
                .layout
                .positions()
                .into_iter()
                .filter(|p| p.place_from == 1 && p.size as usize == teams)
                .collect();
            for p in first {
                let i = p.idx as usize * 2;
                let n = sim.node_mut(p);
                n.teams = [Some(team(i)), Some(team(i + 1))];
                let id = sim.fresh_id();
                sim.node_mut(p).match_id = Some(id);
            }
            sim
        }

        fn fresh_id(&mut self) -> String {
            self.next_id += 1;
            format!("m{}", self.next_id)
        }

        fn node(&self, p: Pos) -> &Node {
            self.b.node(p).unwrap()
        }

        fn node_mut(&mut self, p: Pos) -> &mut Node {
            self.b.nodes.iter_mut().find(|n| n.pos == p).unwrap()
        }

        fn match_of(&self, p: Pos) -> String {
            self.node(p).match_id.clone().unwrap()
        }

        fn apply(&mut self, plan: &Plan) {
            for (p, side, team) in &plan.set {
                let i = match side {
                    Side::A => 0,
                    Side::B => 1,
                };
                self.node_mut(*p).teams[i] = team.clone();
            }
            for id in &plan.withdraw {
                let n = self
                    .b
                    .nodes
                    .iter_mut()
                    .find(|n| n.match_id.as_deref() == Some(id))
                    .unwrap();
                n.match_id = None;
                n.started = false;
                n.winner = None;
            }
            for p in &plan.create {
                let n = self.node(*p);
                assert!(n.match_id.is_none(), "{p:?} already has a match");
                assert!(n.teams.iter().all(Option::is_some), "{p:?} lacks a team");
                let id = self.fresh_id();
                self.node_mut(*p).match_id = Some(id);
            }
            for (p, held) in &plan.held {
                self.node_mut(*p).held = *held;
            }
        }

        /// Record a result (or its undo, `None`) on `p` and apply the plan it gives.
        fn result(&mut self, p: Pos, winner: Option<Side>) -> Plan {
            {
                let n = self.node_mut(p);
                assert!(n.match_id.is_some());
                n.started = true;
                n.winner = winner;
            }
            let plan = self.b.on_result(p);
            self.apply(&plan);
            plan
        }

        fn play(&mut self, list: &[(Pos, Side)]) {
            for (p, side) in list {
                self.result(*p, Some(*side));
            }
        }
    }

    const QF1: Pos = Pos {
        place_from: 1,
        size: 8,
        idx: 0,
    };
    const QF2: Pos = Pos {
        place_from: 1,
        size: 8,
        idx: 1,
    };
    const QF3: Pos = Pos {
        place_from: 1,
        size: 8,
        idx: 2,
    };
    const QF4: Pos = Pos {
        place_from: 1,
        size: 8,
        idx: 3,
    };
    const SF1: Pos = Pos {
        place_from: 1,
        size: 4,
        idx: 0,
    };
    const SF2: Pos = Pos {
        place_from: 1,
        size: 4,
        idx: 1,
    };
    const PS1: Pos = Pos {
        place_from: 5,
        size: 4,
        idx: 0,
    };
    const PS2: Pos = Pos {
        place_from: 5,
        size: 4,
        idx: 1,
    };
    const FINAL: Pos = Pos {
        place_from: 1,
        size: 2,
        idx: 0,
    };
    const THIRD: Pos = Pos {
        place_from: 3,
        size: 2,
        idx: 0,
    };

    #[test]
    fn layout_sizes() {
        for n in SIZES {
            assert!(Layout::new(n, false).is_some(), "{n}");
            assert!(Layout::new(n, true).is_some(), "{n}");
        }
        for n in [0, 2, 3, 6, 12, 64] {
            assert!(Layout::new(n, false).is_none(), "{n}");
            assert!(Layout::new(n, true).is_none(), "{n}");
        }
        let count = |n, full| Layout::new(n, full).unwrap().positions().len();
        assert_eq!((count(8, true), count(8, false)), (12, 8));
        assert_eq!((count(16, true), count(16, false)), (32, 16));
        assert_eq!((count(4, true), count(4, false)), (4, 4));
    }

    #[test]
    fn positions_are_ordered_and_unique() {
        for layout in all_layouts() {
            let ps = layout.positions();
            let mut sorted = ps.clone();
            sorted.sort_by_key(|p| (std::cmp::Reverse(p.size), p.place_from, p.idx));
            assert_eq!(ps, sorted);
            assert_eq!(ps.iter().collect::<HashSet<_>>().len(), ps.len());
        }
    }

    #[test]
    fn targets_for_eight_full() {
        let l = Layout::new(8, true).unwrap();
        assert_eq!(l.winner_target(QF1), Target::Slot(SF1, Side::A));
        assert_eq!(l.loser_target(QF1), Target::Slot(PS1, Side::A));
        assert_eq!(l.winner_target(QF4), Target::Slot(SF2, Side::B));
        assert_eq!(l.loser_target(QF4), Target::Slot(PS2, Side::B));
        let place = |place, place_to| Target::Place { place, place_to };
        assert_eq!(l.winner_target(FINAL), place(1, 1));
        assert_eq!(l.loser_target(FINAL), place(2, 2));
        assert_eq!(l.winner_target(pos(7, 2, 0)), place(7, 7));
        assert_eq!(l.loser_target(pos(7, 2, 0)), place(8, 8));
    }

    #[test]
    fn targets_for_eight_off() {
        let l = Layout::new(8, false).unwrap();
        for qf in [QF1, QF2, QF3, QF4] {
            assert_eq!(
                l.loser_target(qf),
                Target::Place {
                    place: 5,
                    place_to: 8
                }
            );
        }
        assert_eq!(l.loser_target(SF1), Target::Slot(THIRD, Side::A));
        assert_eq!(l.loser_target(SF2), Target::Slot(THIRD, Side::B));
    }

    #[test]
    fn sources_invert_targets() {
        for layout in all_layouts() {
            let ps = layout.positions();
            let set: HashSet<Pos> = ps.iter().copied().collect();
            for &p in &ps {
                for side in [Side::A, Side::B] {
                    let first_round = p.place_from == 1 && p.size == layout.teams;
                    match layout.source(p, side) {
                        None => assert!(first_round, "{layout:?} {p:?} {side:?}"),
                        Some(src) => {
                            assert!(!first_round, "{layout:?} {p:?} {side:?}");
                            let (from, target) = match src {
                                Source::WinnerOf(q) => (q, layout.winner_target(q)),
                                Source::LoserOf(q) => (q, layout.loser_target(q)),
                            };
                            assert!(set.contains(&from), "{from:?} not played");
                            assert_eq!(target, Target::Slot(p, side), "{layout:?} {p:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sources_serialize_as_tagged_positions() {
        let json = serde_json::to_value(Source::WinnerOf(QF1)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"winner_of": {"place_from": 1, "size": 8, "idx": 0}})
        );
        let json = serde_json::to_value(Source::LoserOf(SF2)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"loser_of": {"place_from": 1, "size": 4, "idx": 1}})
        );
    }

    #[test]
    fn labels() {
        let l8 = Layout::new(8, true).unwrap();
        assert_eq!(l8.label(QF1), "Quarter-final 1");
        assert_eq!(l8.label(SF2), "Semi-final 2");
        assert_eq!(l8.label(FINAL), "Final");
        assert_eq!(l8.label(THIRD), "3rd place");
        assert_eq!(l8.label(PS1), "Placement semi 1 (5th–8th)");
        assert_eq!(l8.label(pos(7, 2, 0)), "7th place");
        let l16 = Layout::new(16, true).unwrap();
        assert_eq!(l16.label(pos(1, 16, 2)), "Round of 16, match 3");
        assert_eq!(l16.label(pos(9, 8, 3)), "Placement QF 4 (9th–16th)");
        for layout in all_layouts() {
            for p in layout.positions() {
                let label = layout.label(p);
                assert!(label.chars().count() <= 40, "{label}");
            }
        }
    }

    #[test]
    fn ordinals() {
        let want = [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (22, "22nd"),
            (23, "23rd"),
            (31, "31st"),
            (32, "32nd"),
        ];
        for (n, s) in want {
            assert_eq!(ordinal(n), s);
        }
    }

    #[test]
    fn first_result_waits_for_partner() {
        let mut sim = Sim::new(8, true);
        let plan = sim.result(QF1, Some(Side::A));
        assert_eq!(
            plan,
            Plan {
                set: vec![(SF1, Side::A, Some(team(0))), (PS1, Side::A, Some(team(1)))],
                ..Plan::default()
            }
        );
    }

    #[test]
    fn second_result_creates_worse_places_first() {
        let mut sim = Sim::new(8, false);
        sim.play(&[
            (QF1, Side::A),
            (QF2, Side::A),
            (QF3, Side::A),
            (QF4, Side::A),
            (SF1, Side::A),
        ]);
        let plan = sim.result(SF2, Some(Side::B));
        assert_eq!(plan.create, vec![THIRD, FINAL]);
        assert!(plan.withdraw.is_empty());
    }

    #[test]
    fn undo_withdraws_unstarted_next() {
        let mut sim = Sim::new(8, true);
        sim.play(&[(QF1, Side::A), (QF2, Side::A)]);
        let (sf1, ps1) = (sim.match_of(SF1), sim.match_of(PS1));
        let plan = sim.b.on_result(QF1);
        assert_eq!(plan, Plan::default(), "nothing to do before the undo");
        let plan = sim.result(QF1, None);
        assert_eq!(
            plan,
            Plan {
                set: vec![(SF1, Side::A, None), (PS1, Side::A, None)],
                withdraw: vec![sf1, ps1],
                ..Plan::default()
            }
        );
        // Finishing it again makes both matches again.
        let plan = sim.result(QF1, Some(Side::A));
        assert_eq!(plan.create, vec![PS1, SF1]);
    }

    #[test]
    fn correction_before_next_started_remakes() {
        let mut sim = Sim::new(8, true);
        sim.play(&[(QF1, Side::A), (QF2, Side::A)]);
        let (sf1, ps1) = (sim.match_of(SF1), sim.match_of(PS1));
        let plan = sim.result(QF1, Some(Side::B));
        assert_eq!(
            plan,
            Plan {
                set: vec![(SF1, Side::A, Some(team(1))), (PS1, Side::A, Some(team(0)))],
                withdraw: vec![sf1.clone(), ps1.clone()],
                create: vec![PS1, SF1],
                held: vec![],
            }
        );
        assert_ne!(sim.match_of(SF1), sf1);
        assert_eq!(sim.node(SF1).teams, [Some(team(1)), Some(team(2))]);
    }

    #[test]
    fn correction_after_next_started_holds() {
        let mut sim = Sim::new(8, true);
        sim.play(&[(QF1, Side::A), (QF2, Side::A)]);
        sim.node_mut(SF1).started = true;
        let plan = sim.result(QF1, Some(Side::B));
        assert_eq!(
            plan,
            Plan {
                held: vec![(QF1, true)],
                ..Plan::default()
            }
        );
        assert!(sim.node(QF1).held);
        assert_eq!(sim.node(SF1).teams, [Some(team(0)), Some(team(2))]);
        // Putting the old winner back clears the hold and nothing else.
        let plan = sim.result(QF1, Some(Side::A));
        assert_eq!(
            plan,
            Plan {
                held: vec![(QF1, false)],
                ..Plan::default()
            }
        );
    }

    #[test]
    fn same_winner_correction_changes_nothing() {
        let mut sim = Sim::new(8, true);
        sim.play(&[(QF1, Side::A), (QF2, Side::B)]);
        let plan = sim.result(QF1, Some(Side::A));
        assert_eq!(plan, Plan::default());
    }

    #[test]
    fn replay_discards_exactly_the_subtree() {
        let mut sim = Sim::new(8, true);
        sim.play(&[
            (QF1, Side::A),
            (QF2, Side::A),
            (QF3, Side::A),
            (QF4, Side::A),
            (SF1, Side::A),
            (SF2, Side::A),
            (FINAL, Side::A),
        ]);
        let plan = sim.result(QF1, Some(Side::B));
        assert_eq!(plan.held, vec![(QF1, true)]);
        let ids = |ps: &[Pos]| ps.iter().map(|p| sim.match_of(*p)).collect::<HashSet<_>>();
        let gone = ids(&[SF1, FINAL, THIRD, PS1]);
        let kept = ids(&[QF1, QF2, QF3, QF4, SF2, PS2]);

        let plan = sim.b.replay(QF1);
        assert_eq!(plan.withdraw.iter().cloned().collect::<HashSet<_>>(), gone);
        assert_eq!(plan.withdraw.len(), gone.len());
        assert!(plan.withdraw.iter().all(|id| !kept.contains(id)));
        assert_eq!(sim.b.replay_discards(QF1), vec![SF1, PS1, FINAL, THIRD]);
        assert_eq!(plan.create, vec![PS1, SF1]);
        assert!(plan.held.contains(&(QF1, false)));

        sim.apply(&plan);
        assert!(!sim.node(QF1).held);
        assert_eq!(sim.node(SF1).teams, [Some(team(1)), Some(team(2))]);
        assert_eq!(sim.node(PS1).teams, [Some(team(0)), Some(team(3))]);
        // What SF1 fed lost SF1's team; what SF2 fed kept SF2's.
        assert_eq!(sim.node(FINAL).teams, [None, Some(team(4))]);
        assert_eq!(sim.node(THIRD).teams, [None, Some(team(6))]);
        assert!(sim.node(FINAL).match_id.is_none());
        assert!(sim.node(SF2).match_id.is_some());
        assert!(sim.node(PS2).match_id.is_some());
        // Playing on from here completes the bracket.
        sim.play(&[(SF1, Side::A), (FINAL, Side::B)]);
        assert!(sim.node(THIRD).match_id.is_some());
    }

    #[test]
    fn replay_with_nothing_started_matches_on_result() {
        let mut sim = Sim::new(8, true);
        sim.play(&[(QF1, Side::A), (QF2, Side::A)]);
        sim.node_mut(QF1).winner = Some(Side::B);
        assert_eq!(sim.b.replay(QF1), sim.b.on_result(QF1));
    }

    /// The places handed out by the bracket, as `place -> number of teams`, plus validity.
    fn tally(sim: &Sim, teams: usize) -> BTreeMap<(u32, u32), usize> {
        let places = sim.b.places().expect("bracket complete");
        let ids: Vec<String> = (0..teams).map(team).collect();
        validate_places(&ids, &places).unwrap();
        let mut out = BTreeMap::new();
        for p in places {
            *out.entry((p.place, p.place_to)).or_insert(0) += 1;
        }
        out
    }

    /// Play every ready match, in `positions` order, with `pick` choosing the winner.
    fn play_out(sim: &mut Sim, mut pick: impl FnMut() -> Side) {
        loop {
            let ready = sim
                .b
                .layout
                .positions()
                .into_iter()
                .find(|p| sim.node(*p).match_id.is_some() && sim.node(*p).winner.is_none());
            let Some(p) = ready else { break };
            assert!(sim.b.places().is_none());
            sim.result(p, Some(pick()));
        }
    }

    #[test]
    fn places_for_eight_full_and_off() {
        let mut full = Sim::new(8, true);
        play_out(&mut full, || Side::A);
        let want: BTreeMap<_, _> = (1..=8).map(|i| ((i, i), 1)).collect();
        assert_eq!(tally(&full, 8), want);

        let mut off = Sim::new(8, false);
        play_out(&mut off, || Side::A);
        let want = BTreeMap::from([
            ((1, 1), 1),
            ((2, 2), 1),
            ((3, 3), 1),
            ((4, 4), 1),
            ((5, 8), 4),
        ]);
        assert_eq!(tally(&off, 8), want);
        // Team t0 won every match it played.
        let places = off.b.places().unwrap();
        assert_eq!(
            places.iter().find(|p| p.place == 1).unwrap().team_id,
            team(0)
        );
    }

    #[test]
    fn kept_as_played_places_follow_the_bracket() {
        // Off mode: QF1 is corrected to t1 after SF1 started with t0, and the coordinator
        // keeps it as played. t0 played on, so t1 is the one sharing 5th.
        let mut sim = Sim::new(8, false);
        sim.play(&[(QF1, Side::A), (QF2, Side::A)]);
        sim.node_mut(SF1).started = true;
        sim.result(QF1, Some(Side::B));
        assert!(sim.node(QF1).held);
        sim.node_mut(QF1).held = false;
        play_out(&mut sim, || Side::A);
        tally(&sim, 8);
        let places = sim.b.places().unwrap();
        let of = |t: String| places.iter().find(|p| p.team_id == t).unwrap().clone();
        assert_eq!((of(team(1)).place, of(team(1)).place_to), (5, 8));
        assert_eq!((of(team(0)).place, of(team(0)).place_to), (1, 1));
    }

    #[test]
    fn placed_loser_falls_back_to_the_result() {
        // A winner's slot holding neither of the node's teams (or empty) leaves the result's
        // loser.
        let mut sim = Sim::new(8, false);
        sim.play(&[(QF1, Side::B)]);
        assert_eq!(sim.b.placed_loser(sim.node(QF1)), Some(team(0)));
        sim.node_mut(SF1).teams[0] = Some(team(7));
        assert_eq!(sim.b.placed_loser(sim.node(QF1)), Some(team(0)));
    }

    #[test]
    fn places_wait_for_holds() {
        let mut sim = Sim::new(4, false);
        play_out(&mut sim, || Side::A);
        assert!(sim.b.places().is_some());
        sim.node_mut(SF1).held = true;
        assert!(sim.b.places().is_none());
    }

    #[test]
    fn node_lookups() {
        let sim = Sim::new(4, true);
        let id = sim.match_of(SF1);
        assert_eq!(sim.b.node_by_match(&id).map(|n| n.pos), Some(SF1));
        assert!(sim.b.node_by_match("nope").is_none());
        assert!(sim.b.node(pos(9, 2, 0)).is_none());
    }

    #[test]
    fn unknown_positions_are_harmless() {
        let sim = Sim::new(4, true);
        let bad = pos(9, 2, 0);
        assert_eq!(sim.b.on_result(bad), Plan::default());
        assert_eq!(sim.b.replay(bad), Plan::default());
        assert!(sim.b.replay_discards(bad).is_empty());
        let l = sim.b.layout;
        assert!(l.source(pos(0, 0, 0), Side::A).is_none());
        let _ = l.winner_target(pos(u32::MAX, u32::MAX, u32::MAX));
        let _ = l.loser_target(pos(u32::MAX, u32::MAX, u32::MAX));
        let _ = l.label(pos(u32::MAX, u32::MAX, u32::MAX));
    }

    proptest! {
        #[test]
        fn places_always_tile(
            size in 0usize..4,
            full in any::<bool>(),
            picks in proptest::collection::vec(any::<bool>(), 32),
        ) {
            let teams = SIZES[size];
            let mut sim = Sim::new(teams, full);
            let mut i = 0usize;
            play_out(&mut sim, || {
                i += 1;
                if picks[i % picks.len()] { Side::A } else { Side::B }
            });
            let ids: Vec<String> = (0..teams).map(team).collect();
            let places = sim.b.places();
            prop_assert!(places.is_some());
            prop_assert!(validate_places(&ids, &places.unwrap()).is_ok());
        }

        #[test]
        fn shuffle_is_a_permutation(
            len in 0usize..40,
            values in proptest::collection::vec(any::<u32>(), 0..80),
        ) {
            let mut items: Vec<usize> = (0..len).collect();
            let mut it = values.into_iter();
            // Once the given values run out, `u32::MAX` is never rejected.
            shuffle(&mut items, || it.next().unwrap_or(u32::MAX));
            let mut sorted = items.clone();
            sorted.sort_unstable();
            prop_assert_eq!(sorted, (0..len).collect::<Vec<_>>());
        }
    }

    #[test]
    fn shuffle_is_unbiased_for_three() {
        // Every one of the 3! orders is reached by exactly as many accepted draws.
        let mut counts = BTreeMap::new();
        for a in 10..16u32 {
            for b in 10..16u32 {
                let mut items = [0, 1, 2];
                let mut vals = [a, b].into_iter();
                shuffle(&mut items, || vals.next().unwrap());
                *counts.entry(items).or_insert(0) += 1;
            }
        }
        assert_eq!(counts.len(), 6);
        assert!(counts.values().all(|&c| c == 6), "{counts:?}");
    }
}
