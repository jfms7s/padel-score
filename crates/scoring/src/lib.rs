//! Pure padel scoring engine. No clock, threads, randomness or I/O; builds for wasm32.

mod game;
mod tiebreak;
mod types;

pub use game::{Game, GameScore, PointCall};
pub use tiebreak::{SUPER_TIEBREAK_TO, TIEBREAK_TO, TieBreak};
pub use types::*;
