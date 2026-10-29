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
        // Stable: games arrive in store order (start, formed_at, insertion), which settles ties.
        games.sort_by_key(|g| (g.start, g.formed_at));
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
    games.sort_by_key(|g| (g.start, g.formed_at));
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

/// Where a player who gives their place goes, and who takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Give {
    /// Swap seats with a player of another game that overlaps this one.
    Trade {
        other_game: String,
        other_user: String,
    },
    /// Leave for the front of the line; `replacement` is the line's first member.
    ToLine { replacement: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveRefusal {
    NotInGame,
    /// The game starts inside the fill cutoff.
    TooLate,
    NobodyCanTake,
}

/// Give `user`'s place in the game away. The giver goes to the next game that overlaps this one
/// in (start, formed_at, id) order and that they are free for; there, the highest-priority
/// player (at that game's start) who is free for this game's time moves up into the giver's
/// place. Games already started or inside the fill cutoff are not candidates. With no such
/// game, the giver goes to the front of the line and the line's first member takes the place.
pub fn give_place(
    state: &BoardState,
    game_id: &str,
    user: &str,
    now: i64,
) -> Result<Give, GiveRefusal> {
    let game = state
        .games
        .iter()
        .find(|g| g.id == game_id && g.players.iter().any(|p| p == user))
        .ok_or(GiveRefusal::NotInGame)?;
    let cutoff = now.saturating_add(FILL_CUTOFF_SECS);
    if game.start < cutoff {
        return Err(GiveRefusal::TooLate);
    }
    let mut others: Vec<&Game> = state
        .games
        .iter()
        .filter(|g| {
            g.id != game.id
                && !g.played
                && g.start >= cutoff
                && overlaps(g.start, g.end, game.start, game.end)
                && state.free_for(user, g.start, g.end, Some(&game.id))
        })
        .collect();
    others.sort_by(|a, b| (a.start, a.formed_at, &a.id).cmp(&(b.start, b.formed_at, &b.id)));
    for other in others {
        let taker = other
            .players
            .iter()
            .filter(|p| state.free_for(p, game.start, game.end, Some(&other.id)))
            .filter_map(|p| {
                state
                    .priority_at(p, other.start)
                    .or_else(|| state.priority_at(p, game.start))
            })
            .min();
        if let Some(Priority(_, _, other_user)) = taker {
            return Ok(Give::Trade {
                other_game: other.id.clone(),
                other_user,
            });
        }
    }
    state
        .line(&game.id)
        .into_iter()
        .find(|u| u != user)
        .map(|replacement| Give::ToLine { replacement })
        .ok_or(GiveRefusal::NobodyCanTake)
}

/// Whether `user_a` (in `game_a`) and `user_b` (in `game_b`) can trade places. The games differ
/// and overlap, and neither has been played or started. Each user is in their own game, and
/// each is free for the other game's whole time once they leave their own.
pub fn can_swap(
    state: &BoardState,
    game_a: &str,
    user_a: &str,
    game_b: &str,
    user_b: &str,
    now: i64,
) -> bool {
    let find = |id: &str| state.games.iter().find(|g| g.id == id);
    let (Some(a), Some(b)) = (find(game_a), find(game_b)) else {
        return false;
    };
    a.id != b.id
        && overlaps(a.start, a.end, b.start, b.end)
        && !a.played
        && !b.played
        && a.start > now
        && b.start > now
        && a.players.iter().any(|p| p == user_a)
        && b.players.iter().any(|p| p == user_b)
        && state.free_for(user_a, b.start, b.end, Some(&a.id))
        && state.free_for(user_b, a.start, a.end, Some(&b.id))
}

/// Whether the game can move to `[start, end)`. `"range"` unless it is an hour or an hour and a
/// half on the board's half-hour grid inside its week, and not the time it already has;
/// `"too_soon"` when it starts less than [`LEAD_SECS`] from `now`; `"players_busy"` when one of its players is in another game then.
pub fn check_new_time(
    state: &BoardState,
    game_id: &str,
    start: i64,
    end: i64,
    now: i64,
) -> Result<(), &'static str> {
    let length = end.saturating_sub(start);
    let on_grid = |t: i64| t.saturating_sub(state.starts_at) % SLOT_SECS == 0;
    let ok = (length == MIN_GAME_SECS || length == MAX_GAME_SECS)
        && on_grid(start)
        && on_grid(end)
        && start >= state.starts_at
        && end <= state.ends_at;
    let game = state.games.iter().find(|g| g.id == game_id);
    // Moving a game to the time it already has would only mark it moved and tell everyone.
    let Some(game) = game.filter(|g| ok && (g.start, g.end) != (start, end)) else {
        return Err("range");
    };
    if start < now.saturating_add(LEAD_SECS) {
        return Err("too_soon");
    }
    if game
        .players
        .iter()
        .any(|p| state.busy(p, start, end, Some(game_id)))
    {
        return Err("players_busy");
    }
    Ok(())
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
    fn game_length_is_clamped_to_the_boards_end() {
        // Everyone is free for hours, but the board ends an hour after the first slot: the game
        // gets one hour, not the ninety minutes the cap would allow.
        let mut state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(14, 0));
        state.starts_at = at(10, 0);
        state.ends_at = at(11, 0);
        assert_eq!(
            settle(&state, at(9, 0)),
            vec![form(
                at(10, 0),
                at(11, 0),
                &["ana", "bia", "cris", "duarte"]
            )]
        );
    }

    #[test]
    fn extension_stops_at_a_player_who_is_busy() {
        // Ana plays elsewhere from 11:00, so the first game cannot grow past the hour.
        let mut state = same_range(&["ana", "bia", "cris", "duarte"], at(10, 0), at(14, 0));
        state
            .games
            .push(game("other", at(11, 0), at(12, 0), &["ana", "x", "y", "z"]));
        let steps = settle(&state, T0);
        assert_eq!(
            steps.first(),
            Some(&form(
                at(10, 0),
                at(11, 0),
                &["ana", "bia", "cris", "duarte"]
            ))
        );
    }

    #[test]
    fn priority_uses_the_oldest_of_overlapping_pieces() {
        let state = board(
            &[("ana", 1)],
            vec![
                piece("ana", at(10, 0), at(12, 0), 5),
                piece("ana", at(11, 0), at(13, 0), 2),
            ],
            vec![],
        );
        // Only the first covers 10:30, both cover 11:30: the older one counts.
        assert_eq!(
            state.priority_at("ana", at(10, 30)),
            Some(Priority(5, 1, "ana".to_owned()))
        );
        assert_eq!(
            state.priority_at("ana", at(11, 30)),
            Some(Priority(2, 1, "ana".to_owned()))
        );
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

    /// Two overlapping full games: `g1` 10:00-11:00 and `g2` 10:30-11:30. Everyone covers
    /// their own game; `extra` pieces are added on top.
    fn two_games(extra: Vec<Piece>) -> BoardState {
        let names = ["ana", "bia", "cris", "duarte", "eva", "fio", "gil", "hugo"];
        let members: Vec<(&str, i64)> = names.iter().zip(1..).map(|(n, j)| (*n, j)).collect();
        let mut pieces: Vec<Piece> = names
            .iter()
            .zip(1..)
            .map(|(n, c)| {
                let (s, e) = if ["ana", "bia", "cris", "duarte"].contains(n) {
                    (at(10, 0), at(11, 0))
                } else {
                    (at(10, 30), at(11, 30))
                };
                piece(n, s, e, c)
            })
            .collect();
        pieces.extend(extra);
        let mut g2 = game("g2", at(10, 30), at(11, 30), &["eva", "fio", "gil", "hugo"]);
        g2.formed_at = T0 + 1;
        board(
            &members,
            pieces,
            vec![
                game(
                    "g1",
                    at(10, 0),
                    at(11, 0),
                    &["ana", "bia", "cris", "duarte"],
                ),
                g2,
            ],
        )
    }

    fn extra(id: &str, name: &str, start: i64, end: i64, created_at: i64) -> Piece {
        Piece {
            id: id.to_owned(),
            ..piece(name, start, end, created_at)
        }
    }

    fn trade(other_game: &str, other_user: &str) -> Give {
        Give::Trade {
            other_game: other_game.to_owned(),
            other_user: other_user.to_owned(),
        }
    }

    #[test]
    fn give_trades_with_the_next_overlapping_game() {
        // ana also covers g2; eva (in g2) also covers g1.
        let state = two_games(vec![
            extra("ana-2", "ana", at(11, 0), at(11, 30), 1),
            extra("eva-2", "eva", at(10, 0), at(10, 30), 5),
        ]);
        assert_eq!(give_place(&state, "g1", "ana", T0), Ok(trade("g2", "eva")));
    }

    #[test]
    fn give_does_not_trade_when_the_giver_is_busy_in_the_other_game_time() {
        let covered = || {
            vec![
                extra("ana-2", "ana", at(11, 0), at(11, 30), 1),
                extra("eva-2", "eva", at(10, 0), at(10, 30), 5),
            ]
        };
        assert_eq!(
            give_place(&two_games(covered()), "g1", "ana", T0),
            Ok(trade("g2", "eva"))
        );
        // Ana already plays another game inside g2's time, so she cannot take g2's place.
        let mut state = two_games(covered());
        state
            .games
            .push(game("g3", at(11, 0), at(11, 30), &["ana", "x", "y", "z"]));
        assert_eq!(
            give_place(&state, "g1", "ana", T0),
            Err(GiveRefusal::NobodyCanTake)
        );
    }

    #[test]
    fn give_picks_the_highest_priority_free_player_there() {
        // eva is the oldest in g2 but covers nothing of g1; fio and gil do, fio is older.
        let state = two_games(vec![
            extra("ana-2", "ana", at(11, 0), at(11, 30), 1),
            extra("gil-2", "gil", at(10, 0), at(10, 30), 7),
            extra("fio-2", "fio", at(10, 0), at(10, 30), 6),
        ]);
        assert_eq!(give_place(&state, "g1", "ana", T0), Ok(trade("g2", "fio")));
    }

    #[test]
    fn give_falls_back_to_the_line_and_heads_it() {
        // No overlapping game to go to; eva is free for g1 and not in a game.
        let mut state = two_games(vec![extra("eva-2", "eva", at(10, 0), at(10, 30), 5)]);
        state.games.retain(|g| g.id == "g1");
        assert_eq!(
            give_place(&state, "g1", "ana", T0),
            Ok(Give::ToLine {
                replacement: "eva".to_owned()
            })
        );
    }

    #[test]
    fn give_refused_when_nobody_can_take() {
        let state = two_games(vec![]);
        assert_eq!(
            give_place(&state, "g1", "ana", T0),
            Err(GiveRefusal::NobodyCanTake)
        );
        assert_eq!(
            give_place(&state, "g1", "eva", T0),
            Err(GiveRefusal::NotInGame)
        );
        assert_eq!(
            give_place(&state, "nope", "ana", T0),
            Err(GiveRefusal::NotInGame)
        );
    }

    #[test]
    fn give_refused_inside_the_cutoff() {
        let state = two_games(vec![
            extra("ana-2", "ana", at(11, 0), at(11, 30), 1),
            extra("eva-2", "eva", at(10, 0), at(10, 30), 5),
        ]);
        // g1 starts in 59 minutes.
        assert_eq!(
            give_place(&state, "g1", "ana", at(9, 1)),
            Err(GiveRefusal::TooLate)
        );
        // Exactly an hour out is still fine.
        assert!(give_place(&state, "g1", "ana", at(9, 0)).is_ok());
    }

    #[test]
    fn give_skips_a_trade_game_inside_the_cutoff() {
        // fio (g2, 10:30) gives; g1 (10:00) could take the trade.
        let state = two_games(vec![
            extra("fio-2", "fio", at(10, 0), at(10, 30), 2),
            extra("ana-2", "ana", at(11, 0), at(11, 30), 1),
        ]);
        assert_eq!(
            give_place(&state, "g2", "fio", at(8, 30)),
            Ok(trade("g1", "ana"))
        );
        // At 09:15 g2 is 75 min out but g1 only 45: no trade, and the line is empty.
        assert_eq!(
            give_place(&state, "g2", "fio", at(9, 15)),
            Err(GiveRefusal::NobodyCanTake)
        );
    }

    /// Two overlapping games: g1 10:00-11:00 (ana, bia, cris, duarte) and g2 10:30-11:30
    /// (eva, fio, gui, hugo). Ana covers g2's time too and eva covers g1's.
    fn swap_state() -> BoardState {
        let mut pieces = vec![];
        for (n, c) in [("bia", 2), ("cris", 3), ("duarte", 4)] {
            pieces.push(piece(n, at(10, 0), at(11, 0), c));
        }
        for (n, c) in [("fio", 6), ("gui", 7), ("hugo", 8)] {
            pieces.push(piece(n, at(10, 30), at(11, 30), c));
        }
        pieces.push(piece("ana", at(10, 0), at(11, 30), 1));
        pieces.push(piece("eva", at(10, 0), at(11, 30), 5));
        board(
            &[
                ("ana", 1),
                ("bia", 2),
                ("cris", 3),
                ("duarte", 4),
                ("eva", 5),
                ("fio", 6),
                ("gui", 7),
                ("hugo", 8),
            ],
            pieces,
            vec![
                game(
                    "g1",
                    at(10, 0),
                    at(11, 0),
                    &["ana", "bia", "cris", "duarte"],
                ),
                game("g2", at(10, 30), at(11, 30), &["eva", "fio", "gui", "hugo"]),
            ],
        )
    }

    #[test]
    fn swap_needs_both_free_for_the_other_game() {
        let state = swap_state();
        assert!(can_swap(&state, "g1", "ana", "g2", "eva", T0));
        // Bia is only free for g1.
        assert!(!can_swap(&state, "g1", "bia", "g2", "eva", T0));
        // Fio is only free for g2.
        assert!(!can_swap(&state, "g1", "ana", "g2", "fio", T0));
        // Busy elsewhere: ana also plays a third game inside g2's time.
        let mut busy = swap_state();
        busy.games
            .push(game("g3", at(11, 0), at(12, 0), &["ana", "x", "y", "z"]));
        assert!(!can_swap(&busy, "g1", "ana", "g2", "eva", T0));
        // Not in their game.
        assert!(!can_swap(&state, "g1", "eva", "g2", "ana", T0));
        assert!(!can_swap(&state, "g1", "zed", "g2", "eva", T0));
        // Both games must start after now.
        assert!(!can_swap(&state, "g1", "ana", "g2", "eva", at(10, 0)));
        assert!(can_swap(&state, "g1", "ana", "g2", "eva", at(10, 0) - 1));
    }

    #[test]
    fn swap_refused_for_games_that_do_not_overlap() {
        let mut state = swap_state();
        state.games.retain(|g| g.id == "g1");
        state.games.push(game(
            "g2",
            at(11, 0),
            at(12, 0),
            &["eva", "fio", "gui", "hugo"],
        ));
        state.pieces.push(piece("ana", at(11, 0), at(12, 0), 1));
        state.pieces.push(piece("eva", at(11, 0), at(12, 0), 5));
        // g1 ends exactly when g2 starts: touching is not overlapping.
        assert!(!can_swap(&state, "g1", "ana", "g2", "eva", T0));
        // A game with itself.
        assert!(!can_swap(&state, "g1", "ana", "g1", "bia", T0));
    }

    fn week_state() -> BoardState {
        board(
            &[("ana", 1), ("bia", 2), ("cris", 3), ("duarte", 4)],
            vec![],
            vec![game(
                "g1",
                at(10, 0),
                at(11, 0),
                &["ana", "bia", "cris", "duarte"],
            )],
        )
    }

    #[test]
    fn new_time_must_be_one_hour_to_ninety_minutes_on_the_grid() {
        let state = week_state();
        let check = |s, e| check_new_time(&state, "g1", s, e, T0);
        assert_eq!(check(at(12, 0), at(13, 0)), Ok(()));
        assert_eq!(check(at(12, 0), at(13, 30)), Ok(()));
        assert_eq!(check(at(12, 30), at(13, 30)), Ok(()));
        // Too short, too long, in between, backwards.
        assert_eq!(check(at(12, 0), at(12, 30)), Err("range"));
        assert_eq!(check(at(12, 0), at(14, 0)), Err("range"));
        assert_eq!(check(at(12, 0), at(13, 15)), Err("range"));
        assert_eq!(check(at(13, 0), at(12, 0)), Err("range"));
        // Off the half-hour grid.
        assert_eq!(check(at(12, 15), at(13, 15)), Err("range"));
        // Outside the week.
        assert_eq!(check(T0 - 3600, T0), Err("range"));
        assert_eq!(check(T0 + WEEK, T0 + WEEK + 3600), Err("range"));
        assert_eq!(check(T0 + WEEK - 3600, T0 + WEEK), Ok(()));
        assert_eq!(
            check_new_time(&state, "nope", at(12, 0), at(13, 0), T0),
            Err("range")
        );
    }

    #[test]
    fn new_time_must_differ_from_the_games_own() {
        let state = week_state();
        assert_eq!(
            check_new_time(&state, "g1", at(10, 0), at(11, 0), T0),
            Err("range")
        );
        // The same start with the other length is a change.
        assert_eq!(
            check_new_time(&state, "g1", at(10, 0), at(11, 30), T0),
            Ok(())
        );
    }

    #[test]
    fn time_change_refused_when_a_player_is_busy() {
        let mut state = week_state();
        state
            .games
            .push(game("g2", at(12, 0), at(13, 0), &["duarte", "x", "y", "z"]));
        assert_eq!(
            check_new_time(&state, "g1", at(12, 30), at(13, 30), T0),
            Err("players_busy")
        );
        // Touching the other game is fine, and so is the game's own current time.
        assert_eq!(
            check_new_time(&state, "g1", at(13, 0), at(14, 0), T0),
            Ok(())
        );
        assert_eq!(
            check_new_time(&state, "g1", at(10, 30), at(11, 30), T0),
            Ok(())
        );
    }

    #[test]
    fn new_time_needs_one_hour_lead() {
        let state = week_state();
        let now = at(11, 30);
        assert_eq!(
            check_new_time(&state, "g1", at(12, 0), at(13, 0), now),
            Err("too_soon")
        );
        assert_eq!(
            check_new_time(&state, "g1", at(12, 30), at(13, 30), now),
            Ok(())
        );
    }
}
