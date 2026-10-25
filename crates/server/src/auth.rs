//! The auth core: password hashing, the session cookie, the identity extractors and the CSRF
//! guard.
//!
//! - Passwords are argon2id with the crate's default parameters, hashed and verified on the
//!   blocking pool behind [`AppState::password_permits`] ([`HASH_PERMITS`] at once), so a burst
//!   of logins cannot starve the runtime or exhaust memory. The policy is 8–128 bytes.
//! - A login against an unknown email runs [`verify_dummy`], so it costs the same as a real one.
//! - Secrets at rest are [`crate::tokens::hash_token`]: the store already hashes every session
//!   cookie and device token before it looks one up.
//! - [`CurrentUser`] takes `Authorization: Bearer pdl_…` (a device token) first, then the session
//!   cookie. [`OptionalUser`] reads any failed credential as anonymous. [`SessionUser`] refuses
//!   device tokens, and [`AdminUser`] refuses non-admins.
//! - The CSRF guard runs inside the extractors, because only they know how a request
//!   authenticated: an unsafe method authenticated by the cookie must come from our own origin.
//!   Bearer requests carry no ambient credential and are exempt; routes without an identity
//!   extractor (the phase 2 match routes) never see the guard.

use std::sync::{Arc, OnceLock};

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, Uri, header};
use tokio::sync::Semaphore;

use crate::AppState;
use crate::accounts_store::{DEVICE_TOKEN_PREFIX, SESSION_TTL_SECS, User};
use crate::api::{ApiError, internal, now};
use crate::config::Config;
use crate::tokens::new_token;

/// How many argon2 hashes or verifies run at once.
pub const HASH_PERMITS: usize = 4;
/// The shortest password accepted, in bytes.
pub const MIN_PASSWORD_BYTES: usize = 8;
/// The longest password accepted, in bytes.
pub const MAX_PASSWORD_BYTES: usize = 128;

/// The session cookie's name with `SECURE_COOKIES` on. The `__Host-` prefix makes the browser
/// insist on `Secure`, `Path=/` and no `Domain`, so no sibling subdomain can plant or shadow it.
pub const SECURE_COOKIE_NAME: &str = "__Host-padel_session";
/// The session cookie's name with `SECURE_COOKIES` off (plain-HTTP development).
pub const PLAIN_COOKIE_NAME: &str = "padel_session";

// Passwords.

/// The password policy: 8–128 bytes. A failure is `Validation { field: "password" }`.
pub fn check_password(pw: &str) -> Result<(), ApiError> {
    if (MIN_PASSWORD_BYTES..=MAX_PASSWORD_BYTES).contains(&pw.len()) {
        Ok(())
    } else {
        Err(ApiError::Validation { field: "password" })
    }
}

/// The argon2id PHC string of `pw`, with a fresh salt. It applies [`check_password`] first, so
/// no caller can store a password outside the policy: a failure is
/// `Validation { field: "password" }`.
pub async fn hash_password(permits: &Arc<Semaphore>, pw: &str) -> Result<String, ApiError> {
    check_password(pw)?;
    let pw = pw.to_owned();
    blocking(permits, move || hash_blocking(&pw))
        .await?
        .map_err(internal)
}

/// Whether `pw` matches the PHC string `phc`. A malformed PHC never matches; a failure to run
/// the check is logged and reads as no match.
pub async fn verify_password(permits: &Arc<Semaphore>, pw: &str, phc: &str) -> bool {
    let (pw, phc) = (pw.to_owned(), phc.to_owned());
    blocking(permits, move || verify_blocking(&pw, &phc))
        .await
        .unwrap_or(false)
}

/// Verify `pw` against a dummy hash, for a login whose email is unknown, so it takes as long as
/// a real verify. The dummy is the hash of a random password, made by [`warm_dummy`] at
/// startup (or here, at first use).
pub async fn verify_dummy(permits: &Arc<Semaphore>, pw: &str) {
    let pw = pw.to_owned();
    // The outcome is meaningless (it never matches); only the time spent counts.
    let _ = blocking(permits, move || match dummy_phc() {
        Ok(phc) => verify_blocking(&pw, phc),
        Err(e) => {
            tracing::error!(error = %e, "cannot make the dummy password hash");
            false
        }
    })
    .await;
}

/// Make the dummy hash now, at startup, so the first unknown-email login does not also pay for
/// hashing it and stand out by its timing. An error means the server cannot hash at all; the
/// caller fails closed and stops.
pub async fn warm_dummy(permits: &Arc<Semaphore>) -> Result<(), String> {
    match blocking(permits, || dummy_phc().map(|_| ())).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("cannot make the dummy password hash: {e}")),
        Err(_) => Err("cannot make the dummy password hash".to_owned()),
    }
}

static DUMMY_PHC: OnceLock<String> = OnceLock::new();

/// Called on the blocking pool: the first successful call hashes and keeps the result; a failed
/// hash is never kept, so there is never an empty dummy.
fn dummy_phc() -> Result<&'static str, argon2::password_hash::Error> {
    if let Some(phc) = DUMMY_PHC.get() {
        return Ok(phc);
    }
    let phc = hash_blocking(&new_token())?;
    // A racing first call may have set it already; either hash is as good.
    Ok(DUMMY_PHC.get_or_init(|| phc))
}

fn hash_blocking(pw: &str) -> Result<String, argon2::password_hash::Error> {
    Argon2::default()
        .hash_password(pw.as_bytes())
        .map(|h| h.to_string())
}

fn verify_blocking(pw: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .is_ok_and(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
}

/// Run `work` on the blocking pool while holding one of the hash permits. The permit moves into
/// the task, so a request dropped mid-hash still holds it until the hash ends.
async fn blocking<T, F>(permits: &Arc<Semaphore>, work: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = Arc::clone(permits)
        .acquire_owned()
        .await
        .map_err(internal)?;
    tokio::task::spawn_blocking(move || {
        let out = work();
        drop(permit);
        out
    })
    .await
    .map_err(internal)
}

// The session cookie.

/// The session cookie's name under this config.
pub fn cookie_name(config: &Config) -> &'static str {
    if config.secure_cookies {
        SECURE_COOKIE_NAME
    } else {
        PLAIN_COOKIE_NAME
    }
}

/// The `Set-Cookie` value that stores the session `raw` for 30 days.
pub fn session_cookie(raw: &str, config: &Config) -> String {
    set_cookie(raw, SESSION_TTL_SECS, config)
}

/// The `Set-Cookie` value that deletes the session cookie.
pub fn clear_session_cookie(config: &Config) -> String {
    set_cookie("", 0, config)
}

fn set_cookie(value: &str, max_age: i64, config: &Config) -> String {
    let secure = if config.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}",
        cookie_name(config)
    )
}

/// The raw session cookie value of a request, if it sent one (the first, if several).
pub fn session_token<'a>(headers: &'a HeaderMap, config: &Config) -> Option<&'a str> {
    let name = cookie_name(config);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| {
            let (n, v) = pair.trim().split_once('=')?;
            (n == name).then_some(v)
        })
        .filter(|v| !v.is_empty())
}

/// The `pdl_…` token in `Authorization: Bearer pdl_…` (any scheme case); `None` for any other
/// Authorization value, which is not ours and is ignored.
fn bearer_device_token(value: &HeaderValue) -> Option<&str> {
    let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && token.starts_with(DEVICE_TOKEN_PREFIX))
        .then_some(token)
}

// The CSRF guard.

/// Refuse an unsafe request that is not from our own origin. Called only for requests
/// authenticated by the session cookie.
///
/// `Sec-Fetch-Site` decides when present: `same-origin` or `none` (typed in the address bar)
/// pass, anything else is refused. Without it, `Origin` must equal `PUBLIC_ORIGIN`, or, when
/// that is unset, name the same host and port as the request. With neither header, refuse.
pub(crate) fn check_csrf(
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    config: &Config,
) -> Result<(), ApiError> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(site) = headers.get(SEC_FETCH_SITE) {
        let site = site.to_str().unwrap_or_default();
        let ok = site.eq_ignore_ascii_case("same-origin") || site.eq_ignore_ascii_case("none");
        return if ok { Ok(()) } else { Err(ApiError::Forbidden) };
    }
    let origin = text(header::ORIGIN.as_str()).ok_or(ApiError::Forbidden)?;
    let ok = match &config.public_origin {
        Some(public) => origin.eq_ignore_ascii_case(public),
        None => {
            let host = text(header::HOST.as_str()).or_else(|| uri.authority().map(|a| a.as_str()));
            origin_authority(origin)
                .zip(host)
                .is_some_and(|(o, h)| o.eq_ignore_ascii_case(h))
        }
    };
    if ok { Ok(()) } else { Err(ApiError::Forbidden) }
}

const SEC_FETCH_SITE: &str = "sec-fetch-site";

/// The `host[:port]` of an `http(s)://host[:port]` Origin; `None` for `null` or anything else.
fn origin_authority(origin: &str) -> Option<&str> {
    let (scheme, rest) = origin.split_once("://")?;
    let web = scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http");
    (web && !rest.is_empty() && !rest.contains(['/', '?', '#', '@'])).then_some(rest)
}

// Extractors.

/// How a request proved who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthVia {
    /// The session cookie.
    Session,
    /// A device token, by its `device_tokens.id`.
    Device { token_id: String },
}

/// The signed-in user: a device token or the session cookie, else 401 `unauthorized`.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user: User,
    pub via: AuthVia,
}

/// The signed-in user, or `None` for a missing or failed credential: it never rejects for auth.
/// A store failure is still a 500.
#[derive(Debug, Clone)]
pub struct OptionalUser(pub Option<CurrentUser>);

/// A [`CurrentUser`] signed in by the session cookie; a device token is 403 `forbidden`. Invite,
/// device and admin routes take this, so a watch's token cannot reach them.
#[derive(Debug, Clone)]
pub struct SessionUser(pub CurrentUser);

/// A [`SessionUser`] who is an admin, else 403 `forbidden`.
#[derive(Debug, Clone)]
pub struct AdminUser(pub CurrentUser);

/// The raw secret a request authenticated with. A long-lived stream keeps it to re-check it
/// ([`credential_holds`]); its `Debug` never shows the secret.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    /// The `pdl_…` device token from `Authorization: Bearer`.
    Device(String),
    /// The session cookie's value.
    Session(String),
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Device(_) => f.write_str("Credential::Device(..)"),
            Credential::Session(_) => f.write_str("Credential::Session(..)"),
        }
    }
}

/// The credential a request offers, in the order [`CurrentUser`] tries them: a `pdl_` bearer
/// first (the only one tried when present), then the session cookie.
pub fn credential(headers: &HeaderMap, config: &Config) -> Option<Credential> {
    if let Some(raw) = headers
        .get(header::AUTHORIZATION)
        .and_then(bearer_device_token)
    {
        return Some(Credential::Device(raw.to_owned()));
    }
    session_token(headers, config).map(|raw| Credential::Session(raw.to_owned()))
}

/// Whether `cred` still signs in `user_id` now: not revoked, expired or deleted, and the user
/// not disabled.
pub async fn credential_holds(
    state: &AppState,
    cred: &Credential,
    user_id: &str,
) -> Result<bool, ApiError> {
    let now = now();
    Ok(match cred {
        Credential::Device(raw) => state
            .store
            .token_user(raw, now)
            .await?
            .is_some_and(|t| t.user.id == user_id),
        Credential::Session(raw) => state
            .store
            .session_user(raw, now)
            .await?
            .is_some_and(|u| u.id == user_id),
    })
}

/// The one credential check behind every extractor. A bad `pdl_` bearer is 401 even next to a
/// good cookie: the bearer is the only credential tried when present.
async fn authenticate(parts: &Parts, state: &AppState) -> Result<CurrentUser, ApiError> {
    let now = now();
    match credential(&parts.headers, &state.config).ok_or(ApiError::Unauthorized)? {
        Credential::Device(raw) => {
            let found = state.store.token_user(&raw, now).await?;
            found
                .map(|t| CurrentUser {
                    user: t.user,
                    via: AuthVia::Device {
                        token_id: t.token_id,
                    },
                })
                .ok_or(ApiError::Unauthorized)
        }
        Credential::Session(raw) => {
            check_csrf(&parts.method, &parts.uri, &parts.headers, &state.config)?;
            let user = state
                .store
                .session_user(&raw, now)
                .await?
                .ok_or(ApiError::Unauthorized)?;
            Ok(CurrentUser {
                user,
                via: AuthVia::Session,
            })
        }
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        authenticate(parts, state).await
    }
}

impl FromRequestParts<AppState> for OptionalUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match authenticate(parts, state).await {
            Ok(user) => Ok(OptionalUser(Some(user))),
            // A cross-site unsafe request acts anonymously, never as the user.
            Err(ApiError::Unauthorized | ApiError::Forbidden) => Ok(OptionalUser(None)),
            Err(e) => Err(e),
        }
    }
}

impl FromRequestParts<AppState> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let user = authenticate(parts, state).await?;
        match user.via {
            AuthVia::Session => Ok(SessionUser(user)),
            AuthVia::Device { .. } => Err(ApiError::Forbidden),
        }
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let SessionUser(user) = SessionUser::from_request_parts(parts, state).await?;
        if user.user.is_admin {
            Ok(AdminUser(user))
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use super::*;

    fn permits() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(HASH_PERMITS))
    }

    fn config(secure: &str) -> Config {
        let map = HashMap::from([("SECURE_COOKIES".to_owned(), secure.to_owned())]);
        Config::from_map(&map).unwrap()
    }

    #[tokio::test]
    async fn argon2id_round_trip() {
        let p = permits();
        let phc = hash_password(&p, "correct horse").await.unwrap();
        assert!(phc.starts_with("$argon2id$"), "{phc}");
        assert!(verify_password(&p, "correct horse", &phc).await);
        // A fresh salt every time.
        assert_ne!(phc, hash_password(&p, "correct horse").await.unwrap());
    }

    #[tokio::test]
    async fn hash_password_enforces_the_policy() {
        let p = permits();
        for no in ["", "1234567", &"x".repeat(129)] {
            let r = hash_password(&p, no).await;
            assert!(
                matches!(r, Err(ApiError::Validation { field: "password" })),
                "{} bytes: {r:?}",
                no.len()
            );
        }
    }

    #[tokio::test]
    async fn warm_dummy_makes_the_dummy() {
        warm_dummy(&permits()).await.unwrap();
        assert!(DUMMY_PHC.get().unwrap().starts_with("$argon2id$"));
    }

    #[tokio::test]
    async fn wrong_password_fails() {
        let p = permits();
        let phc = hash_password(&p, "correct horse").await.unwrap();
        assert!(!verify_password(&p, "correct hors", &phc).await);
        assert!(!verify_password(&p, "", &phc).await);
        assert!(!verify_password(&p, "correct horse", "garbage").await);
        assert!(!verify_password(&p, "correct horse", "").await);
    }

    #[tokio::test]
    async fn dummy_verify_takes_real_time() {
        let p = permits();
        // The first call also makes the dummy; time a later one.
        verify_dummy(&p, "first").await;
        assert!(DUMMY_PHC.get().unwrap().starts_with("$argon2id$"));
        let start = Instant::now();
        verify_dummy(&p, "some password").await;
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(5), "took only {elapsed:?}");
    }

    #[test]
    fn password_policy_8_to_128_bytes() {
        for ok in ["12345678", &"x".repeat(128), "ééééé€"] {
            assert!(check_password(ok).is_ok(), "{} bytes", ok.len());
        }
        // Counted in bytes, not characters: 4 two-byte chars are 8 bytes.
        assert!(check_password("éééé").is_ok());
        assert!(check_password("ééé").is_err());
        for no in ["", "1234567", &"x".repeat(129), &"é".repeat(65)] {
            let r = check_password(no);
            assert!(
                matches!(r, Err(ApiError::Validation { field: "password" })),
                "{} bytes: {r:?}",
                no.len()
            );
        }
    }

    #[test]
    fn cookie_name_and_flags_follow_secure_setting() {
        let secure = config("true");
        assert_eq!(cookie_name(&secure), "__Host-padel_session");
        assert_eq!(
            session_cookie("abc", &secure),
            "__Host-padel_session=abc; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax; Secure"
        );
        assert_eq!(
            clear_session_cookie(&secure),
            "__Host-padel_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax; Secure"
        );
        let plain = config("false");
        assert_eq!(cookie_name(&plain), "padel_session");
        assert_eq!(
            session_cookie("abc", &plain),
            "padel_session=abc; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"
        );
        assert_eq!(
            clear_session_cookie(&plain),
            "padel_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"
        );
        for c in [
            session_cookie("abc", &secure),
            session_cookie("abc", &plain),
        ] {
            assert!(!c.to_ascii_lowercase().contains("domain"), "{c}");
        }
        assert_eq!(SESSION_TTL_SECS, 2_592_000);
    }

    #[test]
    fn session_token_reads_only_our_cookie() {
        let c = config("true");
        let mut headers = HeaderMap::new();
        assert_eq!(session_token(&headers, &c), None);
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("a=1; padel_session=plain"),
        );
        assert_eq!(session_token(&headers, &c), None);
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("b=2;__Host-padel_session=raw ; c=3"),
        );
        assert_eq!(session_token(&headers, &c), Some("raw"));
        let mut empty = HeaderMap::new();
        empty.insert(
            header::COOKIE,
            HeaderValue::from_static("__Host-padel_session="),
        );
        assert_eq!(session_token(&empty, &c), None);
    }

    #[test]
    fn bearer_parsing() {
        let parse =
            |v: &'static str| bearer_device_token(&HeaderValue::from_static(v)).map(str::to_owned);
        assert_eq!(parse("Bearer pdl_abc").as_deref(), Some("pdl_abc"));
        assert_eq!(parse("bearer  pdl_abc ").as_deref(), Some("pdl_abc"));
        assert_eq!(parse("Bearer abc"), None);
        assert_eq!(parse("Basic pdl_abc"), None);
        assert_eq!(parse("Bearer"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn origin_authority_parsing() {
        assert_eq!(origin_authority("https://a.test"), Some("a.test"));
        assert_eq!(origin_authority("HTTP://a.test:8080"), Some("a.test:8080"));
        assert_eq!(origin_authority("http://a.test/x"), None);
        assert_eq!(origin_authority("http://u@a.test"), None);
        assert_eq!(origin_authority("http://"), None);
        assert_eq!(origin_authority("null"), None);
        assert_eq!(origin_authority("ftp://a.test"), None);
        assert_eq!(origin_authority("a.test"), None);
    }
}
