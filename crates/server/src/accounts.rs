//! The account routes: first-admin setup and sign-in (login, logout, `me`, password change).
//!
//! - `GET /api/setup` is `{needs_setup}`. `POST /api/setup` takes
//!   `{setup_code, email, display_name, password}` and creates the first admin, signed in: 201
//!   [`Me`] with the session cookie. The code is `SETUP_TOKEN` when set, else the one-time code
//!   [`init_setup_code`] logs at startup while there are no users. A wrong code is 403
//!   `forbidden`; once any user exists it is 409 `setup_done`.
//! - `POST /api/auth/login` takes `{email, password}`: 200 [`Me`] with a NEW session cookie. An
//!   unknown email, a wrong password and a disabled user are the same 401
//!   `invalid_credentials`. Limited per address and per email (see [`crate::ratelimit`]).
//! - `POST /api/auth/logout` ends the presented session and clears the cookie: 204, always.
//! - `GET /api/auth/me` is the signed-in [`Me`], by cookie or device token.
//! - `POST /api/auth/password` takes `{current_password, new_password}`, session only. It signs
//!   the user out everywhere (sessions and device tokens), then signs the caller back in on a
//!   new session: 204. Wrong current passwords are limited like logins, on the email's
//!   `login_email` budget plus a `user:{id}` key.
//!
//! Every response here carries `Referrer-Policy: no-referrer`: they carry or consume secrets.

use std::sync::PoisonError;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::accounts_store::{NewUser, User, normalize_email};
use crate::api::{ApiError, ApiJson, now};
use crate::auth::{
    CurrentUser, SessionUser, check_csrf, check_password, clear_session_cookie, hash_password,
    session_cookie, session_token, verify_dummy, verify_password,
};
use crate::client_ip::{ClientIp, rate_key};
use crate::store::StoreError;
use crate::tokens::{hash_token, new_user_code};

/// The shortest email accepted, in chars.
pub const MIN_EMAIL_CHARS: usize = 3;
/// The longest email accepted, in chars.
pub const MAX_EMAIL_CHARS: usize = 254;
/// The longest display name accepted, in chars, once trimmed.
pub const MAX_DISPLAY_NAME_CHARS: usize = 40;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/setup", get(setup_status).post(setup))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/password", post(change_password))
        .layer(axum::middleware::map_response(no_referrer))
}

async fn no_referrer(mut res: Response) -> Response {
    res.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}

/// Who is signed in, as the PWA sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Me {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
}

impl From<&User> for Me {
    fn from(u: &User) -> Me {
        Me {
            id: u.id.clone(),
            email: u.email.clone(),
            display_name: u.display_name.clone(),
            is_admin: u.is_admin,
        }
    }
}

// The setup code.

/// Make the one-time setup code, store it in `state.setup_code` and log it once at warn level,
/// when the users table is empty and `SETUP_TOKEN` is unset. Returns the code in force (an
/// existing one is kept and not logged again); `None` when setup is done or `SETUP_TOKEN` rules.
pub async fn init_setup_code(state: &AppState) -> Result<Option<String>, StoreError> {
    if state.config.setup_token.is_some() {
        return Ok(None);
    }
    let empty = state.store.user_count().await? == 0;
    let mut slot = state
        .setup_code
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if !empty {
        *slot = None;
        return Ok(None);
    }
    if let Some(code) = slot.as_ref() {
        return Ok(Some(code.clone()));
    }
    let code = new_user_code();
    tracing::warn!("setup code: {code}");
    *slot = Some(code.clone());
    Ok(Some(code))
}

/// A code as typed: uppercased, with dashes and whitespace dropped.
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Whether `given` is the setup code in force, compared in constant time (over the SHA-256 of
/// both normalized codes, so the lengths match too). No code in force matches nothing.
fn setup_code_matches(state: &AppState, given: &str) -> bool {
    let expected = match &state.config.setup_token {
        Some(token) => Some(token.clone()),
        None => state
            .setup_code
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    };
    let Some(expected) = expected.map(|e| normalize_code(&e)) else {
        return false;
    };
    if expected.is_empty() {
        return false;
    }
    let (a, b) = (hash_token(&expected), hash_token(&normalize_code(given)));
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// Validation.

struct ValidUser {
    email: String,
    display_name: String,
}

/// The email (normalized, 3–254 chars, with an `@`), the display name (trimmed, 1–40 chars) and
/// the password policy, checked in that order; a failure names its field.
fn validate_user(email: &str, display_name: &str, password: &str) -> Result<ValidUser, ApiError> {
    let email = normalize_email(email);
    let len = email.chars().count();
    if !(MIN_EMAIL_CHARS..=MAX_EMAIL_CHARS).contains(&len) || !email.contains('@') {
        return Err(ApiError::Validation { field: "email" });
    }
    let display_name = display_name.trim().to_owned();
    if !(1..=MAX_DISPLAY_NAME_CHARS).contains(&display_name.chars().count()) {
        return Err(ApiError::Validation {
            field: "display_name",
        });
    }
    check_password(password)?;
    Ok(ValidUser {
        email,
        display_name,
    })
}

// Responses.

/// `status` with `body` and the cookie that stores the new session `raw`.
fn signed_in(state: &AppState, status: StatusCode, raw: &str, body: Option<Me>) -> Response {
    let cookie = (header::SET_COOKIE, session_cookie(raw, &state.config));
    match body {
        Some(me) => (status, [cookie], Json(me)).into_response(),
        None => (status, [cookie]).into_response(),
    }
}

/// Count one attempt on each limiter in order, stopping at the first refusal (429): a later
/// limiter is never touched once an earlier one refuses, so a refused address opens no email
/// window. The slots already taken stay counted, as a failure's do.
fn acquire_in_order(
    attempts: &[(&crate::ratelimit::RateLimiter, &str)],
    t: i64,
) -> Result<(), ApiError> {
    for (limiter, key) in attempts {
        limiter
            .try_acquire(key, t)
            .map_err(|retry_after| ApiError::TooMany { retry_after })?;
    }
    Ok(())
}

/// Give back every slot [`acquire_in_order`] took at `t`, after a success.
fn refund_all(attempts: &[(&crate::ratelimit::RateLimiter, &str)], t: i64) {
    for (limiter, key) in attempts {
        limiter.refund(key, t);
    }
}

// Setup.

async fn setup_status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let needs_setup = state.store.user_count().await? == 0;
    Ok(Json(json!({ "needs_setup": needs_setup })))
}

#[derive(Deserialize)]
struct SetupBody {
    setup_code: String,
    email: String,
    display_name: String,
    password: String,
}

/// Rate-limited like a login by address (reserve, refund on success). Setup done is 409 before
/// the code is looked at: `GET /api/setup` says so publicly anyway.
async fn setup(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    ApiJson(body): ApiJson<SetupBody>,
) -> Result<Response, ApiError> {
    let t = now();
    let ip_key = rate_key(ip);
    acquire_in_order(&[(&state.limits.login_ip, &ip_key)], t)?;
    if state.store.user_count().await? > 0 {
        return Err(ApiError::Conflict("setup_done"));
    }
    if !setup_code_matches(&state, &body.setup_code) {
        return Err(ApiError::Forbidden);
    }
    let valid = validate_user(&body.email, &body.display_name, &body.password)?;
    let password_hash = hash_password(&state.password_permits, &body.password).await?;
    let user = state
        .store
        .create_first_admin(
            NewUser {
                email: valid.email,
                display_name: valid.display_name,
                password_hash,
            },
            t,
        )
        .await?;
    *state
        .setup_code
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    state.limits.login_ip.refund(&ip_key, t);
    tracing::info!(user_id = %user.id, "first admin created");
    let raw = state.store.create_session(&user.id, now()).await?;
    Ok(signed_in(
        &state,
        StatusCode::CREATED,
        &raw,
        Some(Me::from(&user)),
    ))
}

// Sign-in.

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

/// The recipe of [`crate::ratelimit`]: reserve the address, then (only if it passed) the email;
/// check; refund on success. The
/// email limiter is keyed by the SHA-256 of the normalized email, so an oversized email cannot
/// bloat its memory.
async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, ApiError> {
    let t = now();
    let ip_key = rate_key(ip);
    let email = normalize_email(&body.email);
    let email_key = hash_token(&email);
    let limits = &state.limits;
    let attempts = [
        (&limits.login_ip, ip_key.as_str()),
        (&limits.login_email, email_key.as_str()),
    ];
    acquire_in_order(&attempts, t)?;
    let permits = &state.password_permits;
    let user = match state.store.user_by_email(&email).await? {
        None => {
            verify_dummy(permits, &body.password).await;
            return Err(ApiError::InvalidCredentials);
        }
        Some(found) => {
            // Verified even for a disabled user, so it costs the same as any other answer.
            let ok = verify_password(permits, &body.password, &found.password_hash).await;
            if !ok || found.user.disabled_at.is_some() {
                return Err(ApiError::InvalidCredentials);
            }
            found.user
        }
    };
    refund_all(&attempts, t);
    // Session fixation: never keep a presented session; always mint a new one.
    if let Some(old) = session_token(&headers, &state.config) {
        state.store.delete_session(old).await?;
    }
    let raw = state.store.create_session(&user.id, now()).await?;
    Ok(signed_in(
        &state,
        StatusCode::OK,
        &raw,
        Some(Me::from(&user)),
    ))
}

/// Ends the presented session, if any; a cross-site request carrying the cookie is 403.
async fn logout(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(raw) = session_token(&headers, &state.config) {
        check_csrf(&method, &uri, &headers, &state.config)?;
        state.store.delete_session(raw).await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, clear_session_cookie(&state.config))],
    )
        .into_response())
}

async fn me(user: CurrentUser) -> Json<Me> {
    Json(Me::from(&user.user))
}

#[derive(Deserialize)]
struct PasswordBody {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<AppState>,
    SessionUser(caller): SessionUser,
    ApiJson(body): ApiJson<PasswordBody>,
) -> Result<Response, ApiError> {
    let new_password = |e| match e {
        ApiError::Validation { .. } => ApiError::Validation {
            field: "new_password",
        },
        other => other,
    };
    check_password(&body.new_password).map_err(new_password)?;
    let user_id = &caller.user.id;
    // The login recipe on the email's limiter, so the current-password check is no oracle.
    let t = now();
    let email_key = hash_token(&normalize_email(&caller.user.email));
    let user_key = format!("user:{user_id}");
    let limiter = &state.limits.login_email;
    let attempts = [(limiter, email_key.as_str()), (limiter, user_key.as_str())];
    acquire_in_order(&attempts, t)?;
    let permits = &state.password_permits;
    let current = state
        .store
        .password_hash_by_id(user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if !verify_password(permits, &body.current_password, &current).await {
        return Err(ApiError::InvalidCredentials);
    }
    refund_all(&attempts, t);
    let hash = hash_password(permits, &body.new_password)
        .await
        .map_err(new_password)?;
    state.store.set_password(user_id, &hash, now()).await?;
    let raw = state.store.create_session(user_id, now()).await?;
    tracing::info!(user_id = %user_id, "password changed");
    Ok(signed_in(&state, StatusCode::NO_CONTENT, &raw, None))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn codes_normalize_loosely() {
        assert_eq!(normalize_code(" abcd-efgh "), "ABCDEFGH");
        assert_eq!(normalize_code("ab cd\tef-gh"), "ABCDEFGH");
        assert_eq!(normalize_code("---"), "");
    }

    #[test]
    fn user_validation_names_the_field() {
        let ok = validate_user(" A@B.test ", "  Ana ", "12345678").unwrap();
        assert_eq!(
            (ok.email.as_str(), ok.display_name.as_str()),
            ("a@b.test", "Ana")
        );
        let field = |r: Result<ValidUser, ApiError>| match r {
            Err(ApiError::Validation { field }) => field,
            _ => "ok",
        };
        assert_eq!(field(validate_user("ab", "Ana", "12345678")), "email");
        assert_eq!(field(validate_user("abc", "Ana", "12345678")), "email");
        assert_eq!(field(validate_user("a@b", "", "12345678")), "display_name");
        assert_eq!(field(validate_user("a@b", "Ana", "1234567")), "password");
        // Display names count chars, not bytes.
        assert_eq!(
            field(validate_user("a@b", &"é".repeat(40), "12345678")),
            "ok"
        );
    }
}
