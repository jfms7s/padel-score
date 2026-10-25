//! A team's final place in an event. Task 5 adds the validation and the placings write.

use serde::{Deserialize, Serialize};

/// A place, or a tied range of places (`place` to `place_to`, both included).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub team_id: String,
    pub place: u32,
    pub place_to: u32,
}
