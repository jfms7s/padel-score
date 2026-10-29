//! Guest players on a board: a participant id is a user id or a guest id.

/// Guest ids start with this; user ids are bare UUIDs, so the two never clash.
pub const GUEST_ID_PREFIX: &str = "guest_";

/// Whether a board participant id belongs to a guest.
pub fn is_guest_id(id: &str) -> bool {
    id.starts_with(GUEST_ID_PREFIX)
}

/// A fresh guest id: the prefix and a v4 UUID.
// Task 4 (the guest store API) is the first caller.
#[allow(dead_code)]
pub(crate) fn new_guest_id() -> String {
    format!("{GUEST_ID_PREFIX}{}", uuid::Uuid::new_v4())
}
