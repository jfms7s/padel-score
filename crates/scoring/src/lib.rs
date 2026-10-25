//! Pure padel scoring engine. No clock, threads, randomness or I/O; builds for wasm32.

mod game;
mod types;

pub use game::{Game, GameScore, PointCall};
pub use types::*;
