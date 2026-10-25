use serde::{Deserialize, Serialize};

/// Which side of the court the next serve goes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeSide {
    Right,
    Left,
    ReceiverChooses,
}
