use crate::{Player, Team};
use serde::{Deserialize, Serialize};

/// Which side of the court the next serve goes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeSide {
    Right,
    Left,
    ReceiverChooses,
}

/// The service rotation starting with `first`: `[first, other team's 0, first's partner, other team's 1]`.
pub fn serve_order(first: Player) -> [Player; 4] {
    let other = match first.team {
        Team::A => Team::B,
        Team::B => Team::A,
    };
    let partner = Player {
        team: first.team,
        index: if first.index == 0 { 1 } else { 0 },
    };
    [
        first,
        Player {
            team: other,
            index: 0,
        },
        partner,
        Player {
            team: other,
            index: 1,
        },
    ]
}

/// Points between ends changes in a tie-break or super tie-break.
const TIEBREAK_CHANGE_EVERY: u16 = 6;

/// The player `steps` places along the rotation that starts with `first`.
pub(crate) fn in_rotation(first: Player, steps: u16) -> Player {
    let [p0, p1, p2, p3] = serve_order(first);
    match steps % 4 {
        0 => p0,
        1 => p1,
        2 => p2,
        _ => p3,
    }
}

/// Server of a tie-break after `points_played` points, when `rotation` is the
/// tie-break's first server: one point, then two each, in rotation order.
pub(crate) fn tiebreak_server(first: Player, rotation: u16, points_played: u16) -> Player {
    let offset = points_played.saturating_add(1) / 2;
    in_rotation(first, (rotation % 4).saturating_add(offset % 4))
}

/// Serve from the right after an even number of points, from the left after an odd one;
/// the receivers choose on a deciding (golden or star) point.
pub(crate) fn serve_side(points_played: u16, deciding_point: bool) -> ServeSide {
    if deciding_point {
        ServeSide::ReceiverChooses
    } else if points_played.is_multiple_of(2) {
        ServeSide::Right
    } else {
        ServeSide::Left
    }
}

/// Ends change after a game when the match-wide count of finished games is odd.
pub(crate) fn ends_change_after_game(finished_games: u32) -> bool {
    !finished_games.is_multiple_of(2)
}

/// Ends change in a tie-break (or super tie-break) after every 6 points.
pub(crate) fn ends_change_in_tiebreak(points_played: u16) -> bool {
    points_played > 0 && points_played.is_multiple_of(TIEBREAK_CHANGE_EVERY)
}
