//! The game formation rule, as pure functions over a snapshot of a board.
//!
//! No I/O: the store loads a [`BoardState`], asks [`settle`] what to do and
//! applies the [`Step`]s. Members are always visited in `user_id` order before
//! any priority sort, so the output never depends on the input order.

use super::ranges::{Piece, merged};

/// A slot on the board's grid, in seconds.
pub const SLOT_SECS: i64 = 1800;
pub const MIN_GAME_SECS: i64 = 3600;
pub const MAX_GAME_SECS: i64 = 5400;
/// A game only forms this long before it starts.
pub const LEAD_SECS: i64 = 3600;
/// A short game is only filled up to this long before it starts.
pub const FILL_CUTOFF_SECS: i64 = 3600;
pub const PLACES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    pub joined_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub id: String,
    pub start: i64,
    pub end: i64,
    /// In seat order.
    pub players: Vec<String>,
    /// Users who head this game's line while they stay free, in order.
    pub line_heads: Vec<String>,
    /// Moved by hand: no longer tied to its players' ranges.
    pub moved: bool,
    /// A match was started from it.
    pub played: bool,
    pub formed_at: i64,
}

#[derive(Debug, Clone)]
pub struct BoardState {
    pub starts_at: i64,
    pub ends_at: i64,
    pub members: Vec<Member>,
    pub pieces: Vec<Piece>,
    pub games: Vec<Game>,
}

/// Who goes first: the piece's `created_at`, then `joined_at`, then `user_id`.
/// Smaller means first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub i64, pub i64, pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Fill {
        game_id: String,
        user_id: String,
    },
    Form {
        start: i64,
        end: i64,
        players: Vec<String>,
    },
}

fn overlaps(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

impl BoardState {
    fn pieces_of(&self, user: &str) -> Vec<Piece> {
        self.pieces
            .iter()
            .filter(|p| p.user_id == user)
            .cloned()
            .collect()
    }

    /// Member ids in `user_id` order.
    fn member_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.members.iter().map(|m| m.user_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// The user's priority at `t`, from the piece covering `t` (the oldest one
    /// if several do). `None` for a non-member or when no piece covers `t`.
    pub fn priority_at(&self, user: &str, t: i64) -> Option<Priority> {
        let joined_at = self.members.iter().find(|m| m.user_id == user)?.joined_at;
        let created_at = self
            .pieces
            .iter()
            .filter(|p| p.user_id == user && p.start <= t && t < p.end)
            .map(|p| p.created_at)
            .min()?;
        Some(Priority(created_at, joined_at, user.to_owned()))
    }

    /// Whether the user's pieces cover all of `[start, end)`; touching pieces join.
    pub fn covered(&self, user: &str, start: i64, end: i64) -> bool {
        merged(&self.pieces_of(user))
            .iter()
            .any(|&(s, e)| s <= start && end <= e)
    }

    /// Whether the user plays in any game overlapping `[start, end)`. Moved and
    /// played games count too.
    pub fn busy(&self, user: &str, start: i64, end: i64, except_game: Option<&str>) -> bool {
        self.games.iter().any(|g| {
            Some(g.id.as_str()) != except_game
                && overlaps(g.start, g.end, start, end)
                && g.players.iter().any(|p| p == user)
        })
    }

    /// Covered and not busy.
    pub fn free_for(&self, user: &str, start: i64, end: i64, except_game: Option<&str>) -> bool {
        self.covered(user, start, end) && !self.busy(user, start, end, except_game)
    }

    /// Who is next for a seat in the game: first its line heads that are still
    /// free for it, then every other member free for its whole time, by
    /// priority at its start. Never includes the game's own players.
    pub fn line(&self, game_id: &str) -> Vec<String> {
        let Some(game) = self.games.iter().find(|g| g.id == game_id) else {
            return Vec::new();
        };
        let eligible = |user: &str| {
            self.members.iter().any(|m| m.user_id == user)
                && !game.players.iter().any(|p| p == user)
                && self.free_for(user, game.start, game.end, Some(game_id))
        };
        let mut out: Vec<String> = Vec::new();
        for head in &game.line_heads {
            if !out.contains(head) && eligible(head) {
                out.push(head.clone());
            }
        }
        let mut rest: Vec<Priority> = self
            .member_ids()
            .into_iter()
            .filter(|u| !out.iter().any(|o| o == u) && eligible(u))
            .filter_map(|u| self.priority_at(u, game.start))
            .collect();
        rest.sort();
        out.extend(rest.into_iter().map(|Priority(_, _, user)| user));
        out
    }

    /// Short, unplayed games the fill cutoff still allows, oldest first.
    fn fillable_games(&self, now: i64) -> Vec<&Game> {
        let mut games: Vec<&Game> = self
            .games
            .iter()
            .filter(|g| {
                g.players.len() < PLACES
                    && !g.played
                    && g.start >= now.saturating_add(FILL_CUTOFF_SECS)
            })
            .collect();
        games.sort_by(|a, b| (a.start, a.formed_at, &a.id).cmp(&(b.start, b.formed_at, &b.id)));
        games
    }

    fn next_fill(&self, now: i64) -> Option<(String, String)> {
        self.fillable_games(now).into_iter().find_map(|g| {
            let user = self.line(&g.id).into_iter().next()?;
            Some((g.id.clone(), user))
        })
    }

    /// The four highest priorities among those free for `[t, t + MIN_GAME_SECS)`.
    fn players_at(&self, t: i64) -> Option<Vec<String>> {
        let mut free: Vec<Priority> = self
            .member_ids()
            .into_iter()
            .filter(|u| self.free_for(u, t, t + MIN_GAME_SECS, None))
            .filter_map(|u| self.priority_at(u, t))
            .collect();
        if free.len() < PLACES {
            return None;
        }
        free.sort();
        Some(
            free.into_iter()
                .take(PLACES)
                .map(|Priority(_, _, user)| user)
                .collect(),
        )
    }

    /// The earliest game that can form: `(start, end, players)`.
    fn next_form(&self, now: i64) -> Option<(i64, i64, Vec<String>)> {
        let earliest = now.saturating_add(LEAD_SECS);
        let first = if earliest <= self.starts_at {
            0
        } else {
            (earliest - self.starts_at + SLOT_SECS - 1) / SLOT_SECS
        };
        let mut t = self
            .starts_at
            .saturating_add(first.saturating_mul(SLOT_SECS));
        while t.saturating_add(MIN_GAME_SECS) <= self.ends_at {
            if let Some(players) = self.players_at(t) {
                let cap = (t + MAX_GAME_SECS).min(self.ends_at);
                let mut end = t + MIN_GAME_SECS;
                while end + SLOT_SECS <= cap
                    && players
                        .iter()
                        .all(|p| self.free_for(p, end, end + SLOT_SECS, None))
                {
                    end += SLOT_SECS;
                }
                return Some((t, end, players));
            }
            t += SLOT_SECS;
        }
        None
    }
}

/// What to do to the board now. Works on a copy and repeats until nothing
/// changes: short games fill first, then the earliest possible game forms.
/// Steps come in the order they were applied.
pub fn settle(state: &BoardState, now: i64) -> Vec<Step> {
    let mut work = state.clone();
    let mut steps = Vec::new();
    loop {
        if let Some((game_id, user_id)) = work.next_fill(now) {
            if let Some(game) = work.games.iter_mut().find(|g| g.id == game_id) {
                game.players.push(user_id.clone());
            }
            steps.push(Step::Fill { game_id, user_id });
            continue;
        }
        if let Some((start, end, players)) = work.next_form(now) {
            work.games.push(Game {
                id: format!("settle-{}", steps.len()),
                start,
                end,
                players: players.clone(),
                line_heads: Vec::new(),
                moved: false,
                played: false,
                formed_at: now,
            });
            steps.push(Step::Form {
                start,
                end,
                players,
            });
            continue;
        }
        return steps;
    }
}

/// `(game_id, user_id)` for players of games that are not moved, not played
/// and not started whose pieces no longer cover the game.
pub fn uncovered(state: &BoardState, now: i64) -> Vec<(String, String)> {
    let mut games: Vec<&Game> = state
        .games
        .iter()
        .filter(|g| !g.moved && !g.played && g.start > now)
        .collect();
    games.sort_by(|a, b| (a.start, a.formed_at, &a.id).cmp(&(b.start, b.formed_at, &b.id)));
    games
        .into_iter()
        .flat_map(|g| {
            g.players
                .iter()
                .filter(|p| !state.covered(p, g.start, g.end))
                .map(|p| (g.id.clone(), p.clone()))
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000;
    const WEEK: i64 = 7 * 24 * 3600;

    /// `T0` plus hours and minutes.
    fn at(h: i64, m: i64) -> i64 {
        T0 + h * 3600 + m * 60
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn member(name: &str, joined_at: i64) -> Member {
        Member {
            user_id: name.to_owned(),
            joined_at,
        }
    }

    fn piece(name: &str, start: i64, end: i64, created_at: i64) -> Piece {
        Piece {
            id: format!("{name}-{start}"),
            user_id: name.to_owned(),
            start,
            end,
            created_at,
        }
    }

    fn game(id: &str, start: i64, end: i64, players: &[&str]) -> Game {
        Game {
            id: id.to_owned(),
            start,
            end,
            players: ids(players),
            line_heads: vec![],
            moved: false,
            played: false,
            formed_at: T0,
        }
    }

    fn board(members: &[(&str, i64)], pieces: Vec<Piece>, games: Vec<Game>) -> BoardState {
        BoardState {
            starts_at: T0,
            ends_at: T0 + WEEK,
            members: members.iter().map(|(n, j)| member(n, *j)).collect(),
            pieces,
            games,
        }
    }

    /// Members `names` joined in that order, each free over the same range,
    /// ranges created in that order too.
    fn same_range(names: &[&str], start: i64, end: i64) -> BoardState {
        let members: Vec<(&str, i64)> = names.iter().zip(1..).map(|(n, j)| (*n, j)).collect();
        let pieces = names
            .iter()
            .zip(1..)
            .map(|(n, c)| piece(n, start, end, c))
            .collect();
        board(&members, pieces, vec![])
    }

    fn form(start: i64, end: i64, players: &[&str]) -> Step {
        Step::Form {
            start,
            end,
            players: ids(players),
        }
    }

    fn fill(game_id: &str, user: &str) -> Step {
        Step::Fill {
            game_id: game_id.to_owned(),
            user_id: user.to_owned(),
        }
    }

    /// Applies steps the way the store will: forms become games, fills take a seat.
    fn apply(state: &mut BoardState, steps: &[Step], now: i64) {
        for (n, step) in steps.iter().enumerate() {
            match step {
                Step::Fill { game_id, user_id } => {
                    let g = state.games.iter_mut().find(|g| &g.id == game_id).unwrap();
                    g.players.push(user_id.clone());
                }
                Step::Form {
                    start,
                    end,
                    players,
                } => {
                    let mut g = game(&format!("applied-{n}"), *start, *end, &[]);
                    g.players = players.clone();
                    g.formed_at = now;
                    state.games.push(g);
                }
            }
        }
    }

    #[test]
    fn overlap_of_exactly_one_hour_forms_a_game() {
        let state = board(
            &[("ana", 1), ("bia", 2), ("cris", 3), ("duarte", 4)],
            vec![
                piece("ana", at(9, 0), at(11, 0), 1),
                piece("bia", at(10, 0), at(12, 0), 2),
                piece("cris", at(10, 0), at(11, 0), 3),
                piece("duarte", at(10, 0), at(11, 30), 4),
            ],
            vec![],
        );
        assert_eq!(
            settle(&state, T0),
            vec![form(
                at(10, 0),
                at(11, 0),
                &["ana", "bia", "cris", "duarte"]
            )]
        );
    }

    #[test]
    fn overlap_under_one_hour_forms_nothing() {
        let state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(10, 30));
        assert_eq!(settle(&state, T0), vec![]);
    }

    #[test]
    fn game_is_capped_at_ninety_minutes() {
        let state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(14, 0));
        let p = ["ana", "bia", "cris", "duarte"];
        assert_eq!(
            settle(&state, T0),
            vec![
                form(at(10, 0), at(11, 30), &p),
                form(at(11, 30), at(13, 0), &p),
                form(at(13, 0), at(14, 0), &p),
            ]
        );
    }

    #[test]
    fn three_free_people_form_nothing() {
        let state = same_range(&["ana", "bia", "cris"], at(10, 0), at(14, 0));
        assert_eq!(settle(&state, T0), vec![]);
    }

    #[test]
    fn eight_people_form_two_games_at_the_same_time() {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let state = same_range(&names, at(10, 0), at(11, 0));
        assert_eq!(
            settle(&state, T0),
            vec![
                form(at(10, 0), at(11, 0), &["a", "b", "c", "d"]),
                form(at(10, 0), at(11, 0), &["e", "f", "g", "h"]),
            ]
        );
    }

    #[test]
    fn priority_follows_range_creation_not_join_time() {
        // Eva joined last, but her range was created first. Duarte's was created last.
        let state = board(
            &[
                ("ana", 1),
                ("bia", 2),
                ("cris", 3),
                ("duarte", 4),
                ("eva", 5),
            ],
            vec![
                piece("eva", at(10, 0), at(11, 0), 1),
                piece("ana", at(10, 0), at(11, 0), 2),
                piece("bia", at(10, 0), at(11, 0), 3),
                piece("cris", at(10, 0), at(11, 0), 4),
                piece("duarte", at(10, 0), at(11, 0), 5),
            ],
            vec![],
        );
        assert_eq!(
            settle(&state, T0),
            vec![form(at(10, 0), at(11, 0), &["eva", "ana", "bia", "cris"])]
        );
    }

    #[test]
    fn ties_break_on_join_time_then_user_id() {
        let members = [("a", 2), ("b", 2), ("c", 1), ("d", 1), ("e", 3), ("f", 3)];
        let pieces = members
            .iter()
            .map(|(n, _)| piece(n, at(10, 0), at(11, 0), 7))
            .collect();
        let state = board(&members, pieces, vec![]);
        // c, d joined first; then a, b; e, f are out.
        assert_eq!(
            settle(&state, T0),
            vec![form(at(10, 0), at(11, 0), &["c", "d", "a", "b"])]
        );
    }

    #[test]
    fn earliest_start_wins_over_longest_overlap() {
        let names = ["ana", "bia", "cris", "duarte"];
        let mut pieces = Vec::new();
        for (n, c) in names.iter().zip(1..) {
            pieces.push(piece(n, at(20, 0), at(21, 30), c));
            pieces.push(piece(n, at(18, 0), at(19, 0), c + 10));
        }
        let members: Vec<(&str, i64)> = names.iter().zip(1..).map(|(n, j)| (*n, j)).collect();
        let state = board(&members, pieces, vec![]);
        let steps = settle(&state, T0);
        assert_eq!(
            steps,
            vec![
                form(at(18, 0), at(19, 0), &names),
                form(at(20, 0), at(21, 30), &names),
            ]
        );
    }

    #[test]
    fn lead_time_is_one_hour() {
        let state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(11, 30));
        let p = ["ana", "bia", "cris", "duarte"];
        assert_eq!(
            settle(&state, at(9, 30)),
            vec![form(at(10, 30), at(11, 30), &p)]
        );
        assert_eq!(
            settle(&state, at(9, 0)),
            vec![form(at(10, 0), at(11, 30), &p)]
        );
    }

    fn short_game_board(extra_free: &[&str]) -> BoardState {
        let mut names = vec!["ana", "bia", "cris"];
        names.extend_from_slice(extra_free);
        let mut state = same_range(&names, at(10, 0), at(11, 0));
        state
            .games
            .push(game("g1", at(10, 0), at(11, 0), &["ana", "bia", "cris"]));
        state
    }

    #[test]
    fn fill_cutoff_inside_one_hour() {
        let state = short_game_board(&["eva"]);
        assert_eq!(settle(&state, at(9, 1)), vec![]);
        assert_eq!(settle(&state, at(9, 0)), vec![fill("g1", "eva")]);
    }

    #[test]
    fn game_stays_short_when_line_empty() {
        let state = short_game_board(&[]);
        assert_eq!(settle(&state, T0), vec![]);
    }

    #[test]
    fn short_games_fill_before_new_games_form() {
        // Four more free people: one fills the short game, three are left.
        let state = short_game_board(&["eva", "filipe", "gil", "hugo"]);
        assert_eq!(settle(&state, T0), vec![fill("g1", "eva")]);
    }

    #[test]
    fn played_games_are_never_filled() {
        let mut state = short_game_board(&["eva"]);
        state.games[0].played = true;
        assert_eq!(settle(&state, T0), vec![]);
    }

    #[test]
    fn line_heads_come_first_and_only_while_free() {
        let mut state = short_game_board(&["eva", "filipe"]);
        // Filipe has the lower priority but heads the line.
        state.pieces.retain(|p| p.user_id != "filipe");
        state.pieces.push(piece("filipe", at(10, 0), at(11, 0), 99));
        state.games[0].line_heads = ids(&["filipe", "ghost"]);
        assert_eq!(state.line("g1"), ids(&["filipe", "eva"]));
        // Not free any more: busy in an overlapping game.
        state
            .games
            .push(game("g2", at(10, 0), at(11, 0), &["filipe"]));
        assert_eq!(state.line("g1"), ids(&["eva"]));
    }

    #[test]
    fn existing_games_never_change_when_ranges_are_added() {
        let mut state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(11, 0));
        let steps = settle(&state, T0);
        assert_eq!(steps.len(), 1);
        apply(&mut state, &steps, T0);
        // Four newcomers whose ranges were created earlier than everyone's.
        for (i, n) in ["w", "x", "y", "z"].iter().enumerate() {
            state.members.push(member(n, 10 + i as i64));
            state.pieces.push(piece(n, at(10, 0), at(11, 0), 0));
        }
        let steps = settle(&state, T0);
        assert_eq!(
            steps,
            vec![form(at(10, 0), at(11, 0), &["w", "x", "y", "z"])]
        );
    }

    #[test]
    fn moved_games_are_not_tied_to_ranges() {
        let mut g = game(
            "g1",
            at(10, 0),
            at(11, 0),
            &["ana", "bia", "cris", "duarte"],
        );
        g.moved = true;
        // Their ranges are elsewhere.
        let mut state = same_range(&["ana", "bia", "cris", "duarte"], at(8, 0), at(9, 0));
        state.games.push(g);
        assert_eq!(uncovered(&state, T0), vec![]);

        // Free at the moved time too, they are busy there: a new game leaves them out.
        let mut state = same_range(
            &[
                "ana", "bia", "cris", "duarte", "eva", "filipe", "gil", "hugo",
            ],
            at(10, 0),
            at(11, 0),
        );
        let mut g = game(
            "g1",
            at(10, 0),
            at(11, 0),
            &["ana", "bia", "cris", "duarte"],
        );
        g.moved = true;
        state.games.push(g);
        assert_eq!(
            settle(&state, T0),
            vec![form(
                at(10, 0),
                at(11, 0),
                &["eva", "filipe", "gil", "hugo"]
            )]
        );
    }

    #[test]
    fn uncovered_reports_players_whose_range_was_trimmed() {
        let mut state = same_range(&["ana", "bia", "cris"], at(10, 0), at(11, 0));
        state.members.push(member("duarte", 4));
        state.pieces.push(piece("duarte", at(10, 0), at(10, 30), 4));
        state.games.push(game(
            "g1",
            at(10, 0),
            at(11, 0),
            &["ana", "bia", "cris", "duarte"],
        ));
        assert_eq!(
            uncovered(&state, T0),
            vec![("g1".to_owned(), "duarte".to_owned())]
        );
        // Started, played or moved games are left alone.
        assert_eq!(uncovered(&state, at(10, 0)), vec![]);
        state.games[0].played = true;
        assert_eq!(uncovered(&state, T0), vec![]);
    }

    #[test]
    fn busy_counts_every_game_but_the_excepted_one() {
        let mut state = same_range(&["ana"], at(10, 0), at(14, 0));
        let mut moved = game("g1", at(10, 0), at(11, 0), &["ana"]);
        moved.moved = true;
        let mut played = game("g2", at(12, 0), at(13, 0), &["ana"]);
        played.played = true;
        state.games = vec![moved, played];
        assert!(state.busy("ana", at(10, 30), at(11, 30), None));
        assert!(state.busy("ana", at(12, 30), at(13, 30), None));
        assert!(!state.busy("ana", at(11, 0), at(12, 0), None));
        assert!(!state.busy("ana", at(10, 30), at(11, 30), Some("g1")));
        assert!(!state.free_for("ana", at(10, 0), at(11, 0), None));
        assert!(state.free_for("ana", at(11, 0), at(12, 0), None));
        assert!(!state.free_for("ana", at(13, 30), at(14, 30), None));
    }

    #[test]
    fn touching_pieces_cover_together() {
        let state = board(
            &[("ana", 1)],
            vec![
                piece("ana", at(10, 0), at(11, 0), 1),
                piece("ana", at(11, 0), at(12, 0), 2),
            ],
            vec![],
        );
        assert!(state.covered("ana", at(10, 30), at(11, 30)));
        assert_eq!(
            state.priority_at("ana", at(11, 30)),
            Some(Priority(2, 1, "ana".to_owned()))
        );
        assert_eq!(state.priority_at("ana", at(12, 0)), None);
    }

    /// Hand-written boards: (name, joined, [(start_min, end_min, created)]) with
    /// minutes after `T0`, 30 minute grid.
    type Fixture = &'static [(&'static str, i64, &'static [(i64, i64, i64)])];

    const FIXTURES: [Fixture; 6] = [
        &[
            ("a", 1, &[(600, 840, 1)]),
            ("b", 2, &[(600, 840, 2)]),
            ("c", 3, &[(630, 900, 3)]),
            ("d", 4, &[(600, 780, 4)]),
            ("e", 5, &[(660, 900, 5)]),
        ],
        &[
            ("a", 1, &[(600, 720, 5), (900, 1080, 6)]),
            ("b", 2, &[(600, 720, 4), (900, 1080, 3)]),
            ("c", 3, &[(600, 1080, 2)]),
            ("d", 4, &[(630, 1050, 1)]),
            ("e", 5, &[(600, 660, 7), (960, 1080, 8)]),
            ("f", 6, &[(660, 1080, 9)]),
        ],
        &[
            ("a", 1, &[(600, 1200, 1)]),
            ("b", 1, &[(600, 1200, 1)]),
            ("c", 1, &[(600, 1200, 1)]),
            ("d", 1, &[(600, 1200, 1)]),
            ("e", 1, &[(600, 1200, 1)]),
            ("f", 1, &[(600, 1200, 1)]),
            ("g", 1, &[(600, 1200, 1)]),
            ("h", 1, &[(600, 1200, 1)]),
            ("i", 1, &[(600, 1200, 1)]),
        ],
        &[
            ("a", 3, &[(1200, 1290, 2), (600, 690, 1)]),
            ("b", 2, &[(600, 660, 2), (1200, 1260, 2)]),
            ("c", 1, &[(630, 700, 3)]),
            ("d", 4, &[(600, 690, 4)]),
        ],
        &[
            ("a", 1, &[(0, 300, 9)]),
            ("b", 2, &[(60, 360, 8)]),
            ("c", 3, &[(120, 420, 7)]),
            ("d", 4, &[(180, 480, 6)]),
            ("e", 5, &[(240, 540, 5)]),
            ("f", 6, &[(300, 600, 4)]),
            ("g", 7, &[(360, 660, 3)]),
        ],
        &[
            ("a", 1, &[(1500, 1620, 1)]),
            ("b", 2, &[(1500, 1620, 1)]),
            ("c", 3, &[(1500, 1560, 1)]),
            ("d", 4, &[(1530, 1620, 1)]),
        ],
    ];

    fn from_fixture(fixture: Fixture) -> BoardState {
        let members: Vec<(&str, i64)> = fixture.iter().map(|(n, j, _)| (*n, *j)).collect();
        let mut pieces = Vec::new();
        for (n, _, ranges) in fixture {
            for (s, e, c) in *ranges {
                pieces.push(piece(n, T0 + s * 60, T0 + e * 60, *c));
            }
        }
        board(&members, pieces, vec![])
    }

    #[test]
    fn no_player_in_two_overlapping_games() {
        for (i, fixture) in FIXTURES.iter().enumerate() {
            let mut state = from_fixture(fixture);
            let steps = settle(&state, T0);
            apply(&mut state, &steps, T0);
            for (n, g) in state.games.iter().enumerate() {
                assert_eq!(g.players.len(), 4, "fixture {i}");
                assert!(
                    (MIN_GAME_SECS..=MAX_GAME_SECS).contains(&(g.end - g.start)),
                    "fixture {i}: length"
                );
                for p in &g.players {
                    assert!(
                        state.covered(p, g.start, g.end),
                        "fixture {i}: {p} not free"
                    );
                }
                for other in state.games.iter().skip(n + 1) {
                    if g.start < other.end && other.start < g.end {
                        let clash = g.players.iter().any(|p| other.players.contains(p));
                        assert!(
                            !clash,
                            "fixture {i}: {} and {} share a player",
                            g.id, other.id
                        );
                    }
                }
            }
            // Settled: running again changes nothing.
            assert_eq!(settle(&state, T0), vec![], "fixture {i}: not settled");
        }
        // The busiest fixtures really formed something.
        let mut state = from_fixture(FIXTURES[2]);
        let steps = settle(&state, T0);
        apply(&mut state, &steps, T0);
        assert!(state.games.len() >= 8, "{}", state.games.len());
    }

    #[test]
    fn settle_is_deterministic() {
        let mut state = from_fixture(FIXTURES[1]);
        let mut g = game("g1", at(11, 0), at(12, 0), &["a", "b", "c"]);
        g.line_heads = ids(&["f", "e"]);
        state
            .games
            .push(game("g0", at(30, 0), at(31, 0), &["a", "b"]));
        state.games.push(g);
        let base = settle(&state, T0);
        assert!(!base.is_empty());

        let mut shuffled = state.clone();
        shuffled.members.reverse();
        shuffled.pieces.reverse();
        shuffled.games.reverse();
        assert_eq!(settle(&shuffled, T0), base);

        let mut rotated = state.clone();
        rotated.members.rotate_left(2);
        rotated.pieces.rotate_left(3);
        rotated.games.rotate_left(1);
        assert_eq!(settle(&rotated, T0), base);
    }
}
