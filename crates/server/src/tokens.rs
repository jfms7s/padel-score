//! Secrets: match access tokens and account codes are 32 random bytes, base64url without padding
//! (43 chars). Account secrets are stored only as their SHA-256 ([`hash_token`]).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// A fresh random token.
///
/// Panicking on a missing OS randomness source is deliberate: the server must never hand out a
/// predictable token, and cannot do anything useful without one.
#[allow(clippy::expect_used)]
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The SHA-256 of a raw secret, as lowercase hex. Only this is stored; the raw value is handed
/// out once and never kept.
pub fn hash_token(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(raw.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The unambiguous alphabet of device user codes: no I, O, 0 or 1. It has 32 letters, so a
/// random byte modulo 32 picks one without bias.
pub const USER_CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A fresh device user code, 8 letters from [`USER_CODE_ALPHABET`] shown as `XXXX-XXXX`.
///
/// It is typed by a person, so it is short; it is only valid for minutes, behind a session and a
/// rate limit. Panics like [`new_token`] when the OS random source is missing.
#[allow(clippy::expect_used)]
pub fn new_user_code() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    let mut code = String::with_capacity(9);
    for (i, byte) in bytes.iter().enumerate() {
        if i == 4 {
            code.push('-');
        }
        let letter = USER_CODE_ALPHABET
            .get(usize::from(byte % 32))
            .copied()
            .unwrap_or(b'A');
        code.push(char::from(letter));
    }
    code
}
