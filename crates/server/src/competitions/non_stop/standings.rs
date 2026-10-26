//! Non-stop standings: win 3, draw 1, loss 0; then regulation games won; then head-to-head
//! (a mini-table over the matches among the level teams, re-applied to any smaller group it
//! leaves level); still level, the teams share the place. Pure: no I/O, never panics.

use std::cmp::Reverse;
use std::collections::HashMap;

use padel_scoring::Team;
use serde::Serialize;

use crate::competitions::places::Place;

/// One played match. `games` are regulation games (`match_results.score.regulation_games`),
/// so the overtime game never counts. `winner: None` is a draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub a: String,
    pub b: String,
    pub winner: Option<Team>,
    pub games: [u16; 2],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StandingRow {
    pub team_id: String,
    pub place: u32,
    pub place_to: u32,
    pub played: u32,
    pub won: u32,
    pub drawn: u32,
    pub lost: u32,
    pub games_won: u32,
    pub points: u32,
}

/// Best first; level teams keep `team_ids` order. An outcome naming an unknown team is ignored.
pub fn standings(team_ids: &[String], outcomes: &[Outcome]) -> Vec<StandingRow> {
    let mut rows: Vec<StandingRow> = Vec::new();
    for id in team_ids {
        if rows.iter().all(|r| &r.team_id != id) {
            rows.push(StandingRow {
                team_id: id.clone(),
                place: 0,
                place_to: 0,
                played: 0,
                won: 0,
                drawn: 0,
                lost: 0,
                games_won: 0,
                points: 0,
            });
        }
    }
    let index: HashMap<String, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.team_id.clone(), i))
        .collect();
    for o in outcomes {
        let (Some(&ia), Some(&ib)) = (index.get(&o.a), index.get(&o.b)) else {
            continue;
        };
        if ia == ib {
            continue;
        }
        let a_result = match o.winner {
            Some(Team::A) => Result3::Win,
            Some(Team::B) => Result3::Loss,
            None => Result3::Draw,
        };
        let b_result = match a_result {
            Result3::Win => Result3::Loss,
            Result3::Loss => Result3::Win,
            Result3::Draw => Result3::Draw,
        };
        for (i, result, games) in [(ia, a_result, o.games[0]), (ib, b_result, o.games[1])] {
            if let Some(row) = rows.get_mut(i) {
                row.played += 1;
                row.games_won += u32::from(games);
                match result {
                    Result3::Win => row.won += 1,
                    Result3::Draw => row.drawn += 1,
                    Result3::Loss => row.lost += 1,
                }
                row.points += result.points();
            }
        }
    }
    // Stable: level teams stay in `team_ids` order.
    rows.sort_by_key(|r| (Reverse(r.points), Reverse(r.games_won)));

    let mut sorted = Vec::with_capacity(rows.len());
    let mut next_place = 1u32;
    let mut rest = rows.as_slice();
    while let Some(first) = rest.first() {
        let len = rest
            .iter()
            .take_while(|r| r.points == first.points && r.games_won == first.games_won)
            .count();
        let (group, tail) = rest.split_at(len);
        rest = tail;
        let ids: Vec<&str> = group.iter().map(|r| r.team_id.as_str()).collect();
        for tied in rank_group(&ids, outcomes) {
            let count = u32::try_from(tied.len()).unwrap_or(u32::MAX);
            let place_to = next_place.saturating_add(count).saturating_sub(1);
            for id in tied {
                if let Some(row) = group.iter().find(|r| r.team_id == id) {
                    sorted.push(StandingRow {
                        place: next_place,
                        place_to,
                        ..row.clone()
                    });
                }
            }
            next_place = place_to.saturating_add(1);
        }
    }
    sorted
}

#[derive(Clone, Copy)]
enum Result3 {
    Win,
    Draw,
    Loss,
}

impl Result3 {
    fn points(self) -> u32 {
        match self {
            Self::Win => 3,
            Self::Draw => 1,
            Self::Loss => 0,
        }
    }
}

/// Splits a level group into tie classes, best first, by a mini-table of match points over the
/// matches among its teams. A smaller level subgroup is ranked again the same way; a group the
/// mini-table cannot split stays one class. Teams keep their input order inside a class.
fn rank_group<'a>(ids: &[&'a str], outcomes: &[Outcome]) -> Vec<Vec<&'a str>> {
    if ids.len() < 2 {
        return vec![ids.to_vec()];
    }
    let mut mini: HashMap<&str, u32> = ids.iter().map(|id| (*id, 0)).collect();
    for o in outcomes {
        let (a, b) = (o.a.as_str(), o.b.as_str());
        if a == b || !mini.contains_key(a) || !mini.contains_key(b) {
            continue;
        }
        let (pa, pb) = match o.winner {
            Some(Team::A) => (3, 0),
            Some(Team::B) => (0, 3),
            None => (1, 1),
        };
        if let Some(p) = mini.get_mut(a) {
            *p += pa;
        }
        if let Some(p) = mini.get_mut(b) {
            *p += pb;
        }
    }
    let points = |id: &&str| mini.get(*id).copied().unwrap_or(0);
    let mut ordered = ids.to_vec();
    ordered.sort_by_key(|id| Reverse(points(id)));

    let mut subgroups: Vec<Vec<&str>> = Vec::new();
    for id in ordered {
        match subgroups.last_mut() {
            Some(last) if last.first().map(points) == Some(points(&id)) => last.push(id),
            _ => subgroups.push(vec![id]),
        }
    }
    if subgroups.len() == 1 {
        return subgroups;
    }
    subgroups
        .iter()
        .flat_map(|sub| rank_group(sub, outcomes))
        .collect()
}

/// The places of `rows`, which always tile `1..=n`.
pub fn places_of(rows: &[StandingRow]) -> Vec<Place> {
    rows.iter()
        .map(|r| Place {
            team_id: r.team_id.clone(),
            place: r.place,
            place_to: r.place_to,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::competitions::places::validate_places;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn out(a: &str, b: &str, winner: Option<Team>, games: [u16; 2]) -> Outcome {
        Outcome {
            a: a.into(),
            b: b.into(),
            winner,
            games,
        }
    }

    /// `a` beat `b` 6-0.
    fn beat(a: &str, b: &str) -> Outcome {
        out(a, b, Some(Team::A), [6, 0])
    }

    fn draw(a: &str, b: &str) -> Outcome {
        out(a, b, None, [0, 0])
    }

    fn row<'a>(rows: &'a [StandingRow], id: &str) -> &'a StandingRow {
        rows.iter().find(|r| r.team_id == id).expect("row")
    }

    fn order(rows: &[StandingRow]) -> Vec<&str> {
        rows.iter().map(|r| r.team_id.as_str()).collect()
    }

    fn ranges(rows: &[StandingRow]) -> Vec<(u32, u32)> {
        rows.iter().map(|r| (r.place, r.place_to)).collect()
    }

    fn assert_tiles(teams: &[String], rows: &[StandingRow]) {
        validate_places(teams, &places_of(rows)).expect("places tile");
    }

    type Case = (Vec<String>, Vec<Outcome>);

    fn case_win_draw_loss() -> Case {
        (
            ids(&["a", "b", "c", "d"]),
            vec![
                out("a", "b", Some(Team::A), [6, 3]),
                out("a", "c", None, [4, 4]),
                out("d", "a", Some(Team::B), [2, 6]),
            ],
        )
    }

    /// Rui and Leo end on 7 points with 17 and 15 games won; Leo is listed first.
    fn case_games_split() -> Case {
        (
            ids(&["top", "leo", "rui", "sara", "eva"]),
            vec![
                out("top", "rui", Some(Team::A), [6, 2]),
                out("top", "leo", Some(Team::A), [6, 2]),
                out("top", "sara", Some(Team::A), [6, 0]),
                out("top", "eva", Some(Team::A), [6, 0]),
                out("rui", "leo", None, [4, 4]),
                out("rui", "sara", Some(Team::A), [6, 1]),
                out("rui", "eva", Some(Team::A), [5, 3]),
                out("leo", "sara", Some(Team::A), [6, 1]),
                out("leo", "eva", Some(Team::A), [3, 2]),
                out("sara", "eva", None, [3, 3]),
            ],
        )
    }

    /// `q` and `p` are level on 3 points and 9 games; `p` beat `q`.
    fn case_h2h_pair() -> Case {
        (
            ids(&["q", "p", "y", "z"]),
            vec![
                out("p", "q", Some(Team::A), [6, 3]),
                out("y", "p", Some(Team::A), [6, 3]),
                out("q", "z", Some(Team::A), [6, 3]),
            ],
        )
    }

    /// a, b and c are level on 6 points and 12 games; a > b > c among them.
    fn case_h2h_three() -> Case {
        (
            ids(&["c", "b", "a", "d", "e"]),
            vec![
                beat("a", "b"),
                beat("a", "c"),
                beat("d", "a"),
                beat("e", "a"),
                beat("b", "c"),
                beat("b", "d"),
                beat("e", "b"),
                beat("c", "d"),
                beat("c", "e"),
                draw("d", "e"),
            ],
        )
    }

    fn case_cycle() -> Case {
        (
            ids(&["a", "b", "c"]),
            vec![beat("a", "b"), beat("b", "c"), beat("c", "a")],
        )
    }

    /// a, b, c and d are level on 8 points and 27 games. Among themselves `a` has 6 points,
    /// `b` and `d` 4 and `c` 2, and `b` beat `d`. Listed `d` first, so only the second pass
    /// can put `b` above it.
    fn case_h2h_reapplied() -> Case {
        let mut o = vec![
            out("a", "b", Some(Team::A), [6, 3]),
            out("a", "c", Some(Team::A), [6, 3]),
            out("d", "a", Some(Team::A), [6, 3]),
            out("b", "c", None, [6, 6]),
            out("b", "d", Some(Team::A), [6, 3]),
            out("c", "d", None, [6, 6]),
        ];
        for outsider in ["x", "y"] {
            o.push(out("a", outsider, None, [6, 6]));
        }
        o.push(out("b", "x", Some(Team::A), [6, 0]));
        o.push(out("b", "y", None, [6, 6]));
        o.push(out("c", "x", Some(Team::A), [6, 0]));
        o.push(out("c", "y", Some(Team::A), [6, 0]));
        o.push(out("d", "x", Some(Team::A), [6, 0]));
        o.push(out("d", "y", None, [6, 6]));
        (ids(&["d", "b", "c", "a", "x", "y"]), o)
    }

    fn case_overtime() -> Case {
        (ids(&["a", "b"]), vec![out("a", "b", Some(Team::A), [5, 5])])
    }

    fn case_unplayed() -> Case {
        (ids(&["a", "b", "c", "d"]), Vec::new())
    }

    #[test]
    fn win_draw_loss_points() {
        let (teams, o) = case_win_draw_loss();
        let rows = standings(&teams, &o);
        let a = row(&rows, "a");
        assert_eq!(
            (a.played, a.won, a.drawn, a.lost, a.points),
            (3, 2, 1, 0, 7)
        );
        assert_eq!(a.games_won, 6 + 4 + 6);
        let b = row(&rows, "b");
        assert_eq!(
            (b.played, b.won, b.drawn, b.lost, b.points),
            (1, 0, 0, 1, 0)
        );
        let c = row(&rows, "c");
        assert_eq!(
            (c.played, c.won, c.drawn, c.lost, c.points),
            (1, 0, 1, 0, 1)
        );
        assert_eq!(order(&rows), ["a", "c", "b", "d"]);
        assert_eq!(ranges(&rows), [(1, 1), (2, 2), (3, 3), (4, 4)]);
    }

    #[test]
    fn games_split_level_points() {
        let (teams, o) = case_games_split();
        let rows = standings(&teams, &o);
        let (rui, leo) = (row(&rows, "rui"), row(&rows, "leo"));
        assert_eq!((rui.points, rui.games_won), (7, 17));
        assert_eq!((leo.points, leo.games_won), (7, 15));
        assert_eq!((rui.place, rui.place_to), (2, 2));
        assert_eq!((leo.place, leo.place_to), (3, 3));
        assert_eq!(order(&rows).first(), Some(&"top"));
    }

    #[test]
    fn head_to_head_splits_level_games() {
        let (teams, o) = case_h2h_pair();
        let rows = standings(&teams, &o);
        let (p, q) = (row(&rows, "p"), row(&rows, "q"));
        assert_eq!((p.points, p.games_won), (3, 9));
        assert_eq!((q.points, q.games_won), (3, 9));
        assert_eq!(order(&rows), ["p", "q", "y", "z"]);
        assert_eq!(ranges(&rows), [(1, 1), (2, 2), (3, 3), (4, 4)]);
    }

    #[test]
    fn head_to_head_mini_table_for_three() {
        let (teams, o) = case_h2h_three();
        let rows = standings(&teams, &o);
        assert_eq!(order(&rows), ["e", "a", "b", "c", "d"]);
        assert_eq!(ranges(&rows), [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
        let a = row(&rows, "a");
        assert_eq!((a.points, a.games_won), (6, 12));
    }

    #[test]
    fn head_to_head_cycle_shares_one_range() {
        let (teams, o) = case_cycle();
        let rows = standings(&teams, &o);
        assert_eq!(order(&rows), ["a", "b", "c"]);
        assert_eq!(ranges(&rows), [(1, 3), (1, 3), (1, 3)]);
    }

    #[test]
    fn head_to_head_reapplied_to_remaining_pair() {
        let (teams, o) = case_h2h_reapplied();
        let rows = standings(&teams, &o);
        for id in ["a", "b", "c", "d"] {
            let r = row(&rows, id);
            assert_eq!((r.points, r.games_won), (8, 27));
        }
        assert_eq!(order(&rows).get(..4), Some(&["a", "b", "d", "c"][..]));
        assert_eq!(
            ranges(&rows).get(..4),
            Some(&[(1, 1), (2, 2), (3, 3), (4, 4)][..])
        );
    }

    #[test]
    fn overtime_counts_regulation_games_only() {
        let (teams, o) = case_overtime();
        let rows = standings(&teams, &o);
        let (a, b) = (row(&rows, "a"), row(&rows, "b"));
        assert_eq!((a.points, a.games_won), (3, 5));
        assert_eq!((b.points, b.games_won), (0, 5));
    }

    #[test]
    fn unplayed_teams_share_last() {
        let (teams, o) = case_unplayed();
        let rows = standings(&teams, &o);
        assert_eq!(order(&rows), ["a", "b", "c", "d"]);
        assert_eq!(ranges(&rows), vec![(1, 4); 4]);
        assert!(rows.iter().all(|r| r.played == 0 && r.points == 0));
    }

    #[test]
    fn unknown_teams_are_ignored() {
        let teams = ids(&["a", "b"]);
        let o = [beat("a", "zed"), beat("a", "a"), beat("b", "a")];
        let rows = standings(&teams, &o);
        assert_eq!(order(&rows), ["b", "a"]);
        assert_eq!(row(&rows, "a").played, 1);
    }

    #[test]
    fn places_of_tiles() {
        for (teams, o) in [
            case_win_draw_loss(),
            case_games_split(),
            case_h2h_pair(),
            case_h2h_three(),
            case_cycle(),
            case_h2h_reapplied(),
            case_overtime(),
            case_unplayed(),
        ] {
            let rows = standings(&teams, &o);
            assert_eq!(rows.len(), teams.len());
            assert_tiles(&teams, &rows);
        }
    }
}
