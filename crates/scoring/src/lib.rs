//! Pure padel scoring engine. No clock, threads, randomness or I/O; builds for wasm32.

mod final_score;
mod game;
mod serve;
mod state;
mod tiebreak;
mod types;

pub use final_score::{FinalScore, InvalidScore, events_for_final, validate_final};
pub use game::{Game, GameScore, PointCall};
pub use serve::{ServeSide, serve_order};
pub use state::{Current, MatchState, Score, SetScore, Status};
pub use tiebreak::{SUPER_TIEBREAK_TO, TIEBREAK_TO, TieBreak};
pub use types::*;

/// Play `events` from the start and return the resulting score.
///
/// Invalid rules or start are reported at index 0; otherwise `index` is the first rejected event.
///
/// The server resolves an undo by removing the target event from its log and replaying, so the
/// engine sees only `Point` and `TimeUp`.
pub fn replay(format: &Format, start: &Start, events: &[Event]) -> Result<Score, ReplayError> {
    let mut m = MatchState::new(*format, *start).map_err(|e| ReplayError {
        index: 0,
        kind: ErrorKind::InvalidRules(e),
    })?;
    for (index, e) in events.iter().enumerate() {
        m.apply(*e).map_err(|kind| ReplayError { index, kind })?;
    }
    Ok(m.score())
}
