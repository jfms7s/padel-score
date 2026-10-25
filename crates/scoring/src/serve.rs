use crate::{Player, Start};
use serde::{Deserialize, Serialize};

/// Which side of the court the next serve goes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeSide {
    Right,
    Left,
    ReceiverChooses,
}

/// The service rotation for `start`: `[first, receiving team's chosen first server, first's partner,
/// receiving team's other player]`. Each pair chooses its own first server (FIP).
pub fn serve_order(start: &Start) -> [Player; 4] {
    let first = start.first_server;
    let other = |index: u8| Player {
        team: first.team.other(),
        index,
    };
    let partner = Player {
        team: first.team,
        index: 1u8.saturating_sub(first.index),
    };
    let receiver_first = start.other_first_server;
    [
        first,
        other(receiver_first),
        partner,
        other(1u8.saturating_sub(receiver_first)),
    ]
}

/// Points between ends changes in a tie-break or super tie-break.
const TIEBREAK_CHANGE_EVERY: u16 = 6;

/// The player `steps` places along the rotation of `start`.
pub(crate) fn in_rotation(start: &Start, steps: u16) -> Player {
    let [p0, p1, p2, p3] = serve_order(start);
    match steps % 4 {
        0 => p0,
        1 => p1,
        2 => p2,
        _ => p3,
    }
}

/// Server of a tie-break after `points_played` points, when `rotation` is the
/// tie-break's first server: one point, then two each, in rotation order.
pub(crate) fn tiebreak_server(start: &Start, rotation: u16, points_played: u16) -> Player {
    let offset = points_played.saturating_add(1) / 2;
    in_rotation(start, (rotation % 4).saturating_add(offset % 4))
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

/// Ends change after a game when the games played in its set add up to an odd number
/// (FIP Rule 5.1). A set's tie-break counts as its 13th game.
pub(crate) fn ends_change_after_game(games_in_set: u16) -> bool {
    !games_in_set.is_multiple_of(2)
}

/// Ends change in a tie-break (or super tie-break) after every 6 points.
pub(crate) fn ends_change_in_tiebreak(points_played: u16) -> bool {
    points_played > 0 && points_played.is_multiple_of(TIEBREAK_CHANGE_EVERY)
}
