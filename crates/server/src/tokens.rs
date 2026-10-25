//! Match access tokens: 32 random bytes, base64url without padding (43 chars).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// A fresh random token.
///
/// Aborting on a missing OS randomness source is deliberate: the server must never hand out a
/// predictable token, and cannot do anything useful without one.
#[allow(clippy::expect_used)]
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}
