//! The accounts store: users, sessions, invites, password resets, the device-code flow and
//! device tokens, plus the owned-match reads.
//!
//! These are methods on [`Store`], not a second store type: one handle in `AppState`, over one
//! pool. They live in their own module so `store.rs` stays about matches. The rules are the same
//! as there: the clock is never read here (callers pass `now` in unix seconds), and every write
//! that checks state before it consumes or mints runs in [`Store::begin`]'s `BEGIN IMMEDIATE`
//! transaction, so two requests cannot both pass the check.
//!
//! Every secret (session, invite, reset and device codes, device tokens) comes from
//! [`new_token`], is returned to the caller once, and is stored only as its [`hash_token`].
//! Emails are trimmed and lowercased here, at the store boundary.

use std::fmt;

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::store::{Store, StoreError, StoredMatch, match_from_row, select_matches};
use crate::tokens::{USER_CODE_ALPHABET, hash_token, new_token, new_user_code};

/// A session lasts 30 days.
pub const SESSION_TTL_SECS: i64 = 30 * 86_400;
/// An invite lasts 7 days.
pub const INVITE_TTL_SECS: i64 = 7 * 86_400;
/// A password reset code lasts 24 hours.
pub const RESET_TTL_SECS: i64 = 86_400;
/// A device code lasts 10 minutes.
pub const DEVICE_CODE_TTL_SECS: i64 = 600;
/// A device polling again sooner than this is told to slow down.
pub const DEVICE_POLL_INTERVAL_SECS: i64 = 5;
/// Unused, unrevoked, unexpired invites one user may hold at once.
pub const MAX_ACTIVE_INVITES: i64 = 20;
/// A device token's `last_used_at` is written at most this often.
pub const LAST_USED_RESOLUTION_SECS: i64 = 60;
/// How many owned matches [`Store::live_matches_for_owner`] returns as candidates.
pub const OWNED_MATCH_CANDIDATES: i64 = 200;
/// Device tokens start with this, so a Bearer token is recognisable as one.
pub const DEVICE_TOKEN_PREFIX: &str = "pdl_";
/// The leading chars of a device token kept in clear, for the device list.
const DISPLAY_PREFIX_CHARS: usize = 8;

/// Trim and lowercase. SQLite's NOCASE only folds ASCII, so this is done in the app.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A typed user code in its stored form `XXXX-XXXX`: uppercased, with spaces and dashes
/// dropped and the dash put back. `None` unless it is exactly 8 letters of the alphabet.
pub fn normalize_user_code(input: &str) -> Option<String> {
    let letters: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if letters.len() != 8 || !letters.bytes().all(|b| USER_CODE_ALPHABET.contains(&b)) {
        return None;
    }
    let (first, second) = letters.split_at_checked(4)?;
    Some(format!("{first}-{second}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub is_admin: bool,
    pub disabled_at: Option<i64>,
    pub invited_by: Option<String>,
    pub created_at: i64,
}

/// A user with their password hash, for sign-in only. No `Debug`, so the hash stays out of logs.
pub struct UserWithHash {
    pub user: User,
    pub password_hash: String,
}

/// A user to create. The password is already hashed; the email is normalized on insert.
pub struct NewUser {
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteStatus {
    Valid,
    Used,
    Expired,
    /// Revoked by its creator, or its creator is now disabled.
    Revoked,
}

impl InviteStatus {
    /// Used wins over revoked, and revoked over expired.
    fn of(used_at: Option<i64>, revoked: bool, expires_at: i64, now: i64) -> InviteStatus {
        if used_at.is_some() {
            InviteStatus::Used
        } else if revoked {
            InviteStatus::Revoked
        } else if expires_at <= now {
            InviteStatus::Expired
        } else {
            InviteStatus::Valid
        }
    }

    /// The 409 code an accept of an invite in this state gets; `None` when it is valid.
    fn conflict(self) -> Option<&'static str> {
        match self {
            InviteStatus::Valid => None,
            InviteStatus::Used => Some("invite_used"),
            InviteStatus::Expired => Some("invite_expired"),
            InviteStatus::Revoked => Some("invite_revoked"),
        }
    }
}

/// A freshly minted invite. `code` is the raw secret, shown once.
#[derive(Clone, PartialEq, Eq)]
pub struct CreatedInvite {
    pub id: String,
    pub code: String,
    pub expires_at: i64,
}

/// What the public invite page may show about a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteInfo {
    pub id: String,
    pub status: InviteStatus,
    pub created_by_name: String,
    pub expires_at: i64,
}

/// One of a user's own invites, for their list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteSummary {
    pub id: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    pub used_by_name: Option<String>,
    pub revoked_at: Option<i64>,
}

impl InviteSummary {
    pub fn status(&self, now: i64) -> InviteStatus {
        InviteStatus::of(
            self.used_at,
            self.revoked_at.is_some(),
            self.expires_at,
            now,
        )
    }
}

/// A started device flow. `device_code` is the raw secret the watch polls with.
#[derive(Clone, PartialEq, Eq)]
pub struct StartedDevice {
    pub device_code: String,
    pub user_code: String,
    pub expires_at: i64,
}

/// A device code as the approval page sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub id: String,
    pub user_code: String,
    pub device_name: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub approved_at: Option<i64>,
    pub denied_at: Option<i64>,
    pub consumed_at: Option<i64>,
}

impl DeviceCode {
    /// Still waiting for a decision, and not expired.
    pub fn is_pending(&self, now: i64) -> bool {
        self.approved_at.is_none() && self.denied_at.is_none() && self.expires_at > now
    }
}

/// A device token's owner, as [`Store::token_user`] finds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUser {
    /// The `device_tokens.id` (not the secret), for revoking or naming this device.
    pub token_id: String,
    pub user: User,
}

/// The outcome of one device poll (RFC 8628 §3.5).
#[derive(Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Expired,
    Denied,
    /// Approved: the device token, minted by this poll and never again.
    Issued {
        raw_token: String,
    },
    /// The token was already issued by an earlier poll.
    Consumed,
}

/// A signed-in device, for the user's device list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceToken {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// Rows removed by one [`Store::purge_expired_auth`] pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgedAuth {
    pub sessions: u64,
    pub invites: u64,
    pub device_codes: u64,
    pub resets: u64,
}

impl PurgedAuth {
    pub fn total(&self) -> u64 {
        self.sessions + self.invites + self.device_codes + self.resets
    }
}

// Raw secrets never reach a log line: their `Debug` output shows this instead.
const REDACTED: &str = "<redacted>";

impl fmt::Debug for CreatedInvite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedInvite")
            .field("id", &self.id)
            .field("code", &REDACTED)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl fmt::Debug for StartedDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartedDevice")
            .field("device_code", &REDACTED)
            .field("user_code", &self.user_code)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl fmt::Debug for PollOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PollOutcome::Pending => f.write_str("Pending"),
            PollOutcome::SlowDown => f.write_str("SlowDown"),
            PollOutcome::Expired => f.write_str("Expired"),
            PollOutcome::Denied => f.write_str("Denied"),
            PollOutcome::Issued { .. } => f
                .debug_struct("Issued")
                .field("raw_token", &REDACTED)
                .finish(),
            PollOutcome::Consumed => f.write_str("Consumed"),
        }
    }
}

/// The `users` columns, read from a table aliased `u`.
macro_rules! user_columns {
    () => {
        "u.id, u.email, u.display_name, u.is_admin, u.disabled_at, u.invited_by, u.created_at"
    };
}

fn user_from_row(row: &SqliteRow) -> Result<User, sqlx::Error> {
    Ok(User {
        id: row.try_get("id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        is_admin: row.try_get("is_admin")?,
        disabled_at: row.try_get("disabled_at")?,
        invited_by: row.try_get("invited_by")?,
        created_at: row.try_get("created_at")?,
    })
}

fn device_code_from_row(row: &SqliteRow) -> Result<DeviceCode, sqlx::Error> {
    Ok(DeviceCode {
        id: row.try_get("id")?,
        user_code: row.try_get("user_code")?,
        device_name: row.try_get("device_name")?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        approved_at: row.try_get("approved_at")?,
        denied_at: row.try_get("denied_at")?,
        consumed_at: row.try_get("consumed_at")?,
    })
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn insert_user_in(
    conn: &mut SqliteConnection,
    new: NewUser,
    is_admin: bool,
    invited_by: Option<&str>,
    now: i64,
) -> Result<User, StoreError> {
    let user = User {
        id: new_id(),
        email: normalize_email(&new.email),
        display_name: new.display_name,
        is_admin,
        disabled_at: None,
        invited_by: invited_by.map(str::to_owned),
        created_at: now,
    };
    let inserted = sqlx::query(
        "INSERT INTO users (id, email, display_name, password_hash, is_admin, invited_by, \
         created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.email)
    .bind(&user.display_name)
    .bind(&new.password_hash)
    .bind(is_admin)
    .bind(&user.invited_by)
    .bind(now)
    .execute(conn)
    .await;
    match inserted {
        Ok(_) => Ok(user),
        Err(e) if is_unique_violation(&e) => Err(StoreError::Conflict("email_taken")),
        Err(e) => Err(e.into()),
    }
}

/// End every way the user is signed in: delete their sessions, revoke their device tokens, and
/// deny their approved device codes that have not been polled yet.
async fn revoke_credentials_in(
    conn: &mut SqliteConnection,
    user_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE device_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
        .bind(now)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    // An approved code not yet polled would otherwise mint a token after this.
    sqlx::query(
        "UPDATE device_codes SET denied_at = ? \
         WHERE approved_user_id = ? AND consumed_at IS NULL AND denied_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Mark every unused reset code of the user used, once their password has been set.
async fn invalidate_resets_in(
    conn: &mut SqliteConnection,
    user_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE password_resets SET used_at = ? WHERE user_id = ? AND used_at IS NULL")
        .bind(now)
        .bind(user_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// The invite behind a raw code, with what decides its status: (id, creator id, status, creator
/// display name, expires_at).
async fn invite_in(
    conn: &mut SqliteConnection,
    raw: &str,
    now: i64,
) -> Result<Option<(String, String, InviteStatus, String, i64)>, StoreError> {
    let row = sqlx::query(
        "SELECT i.id, i.created_by, i.expires_at, i.used_at, i.revoked_at, u.display_name, \
         u.disabled_at FROM invites i JOIN users u ON u.id = i.created_by WHERE i.code_hash = ?",
    )
    .bind(hash_token(raw))
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let expires_at: i64 = row.try_get("expires_at")?;
    let revoked = row.try_get::<Option<i64>, _>("revoked_at")?.is_some()
        || row.try_get::<Option<i64>, _>("disabled_at")?.is_some();
    let status = InviteStatus::of(row.try_get("used_at")?, revoked, expires_at, now);
    Ok(Some((
        row.try_get("id")?,
        row.try_get("created_by")?,
        status,
        row.try_get("display_name")?,
        expires_at,
    )))
}

async fn device_code_by_user_code_in(
    conn: &mut SqliteConnection,
    user_code: &str,
) -> Result<Option<DeviceCode>, StoreError> {
    let Some(code) = normalize_user_code(user_code) else {
        return Ok(None);
    };
    let row = sqlx::query(
        "SELECT id, user_code, device_name, created_at, expires_at, approved_at, denied_at, \
         consumed_at FROM device_codes WHERE user_code = ?",
    )
    .bind(code)
    .fetch_optional(conn)
    .await?;
    Ok(row.as_ref().map(device_code_from_row).transpose()?)
}

impl Store {
    // Users.

    pub async fn user_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(self.pool())
            .await?)
    }

    /// A regular (non-admin) user with no inviter. A taken email is `Conflict("email_taken")`.
    pub async fn create_user(&self, new: NewUser, now: i64) -> Result<User, StoreError> {
        let mut conn = self.pool().acquire().await?;
        insert_user_in(&mut conn, new, false, None, now).await
    }

    /// The first account, an admin. Refused with `Conflict("setup_done")` once any user exists;
    /// the check and the insert share one IMMEDIATE transaction, so only one setup can win.
    pub async fn create_first_admin(&self, new: NewUser, now: i64) -> Result<User, StoreError> {
        let mut tx = self.begin().await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&mut *tx)
            .await?;
        if count > 0 {
            return Err(StoreError::Conflict("setup_done"));
        }
        let user = insert_user_in(&mut tx, new, true, None, now).await?;
        tx.commit().await?;
        Ok(user)
    }

    pub async fn user_by_email(&self, email: &str) -> Result<Option<UserWithHash>, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            ", u.password_hash FROM users u WHERE u.email = ?"
        ))
        .bind(normalize_email(email))
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| -> Result<UserWithHash, sqlx::Error> {
            Ok(UserWithHash {
                user: user_from_row(&row)?,
                password_hash: row.try_get("password_hash")?,
            })
        })
        .transpose()
        .map_err(StoreError::from)
    }

    pub async fn user_by_id(&self, id: &str) -> Result<Option<User>, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            " FROM users u WHERE u.id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(user_from_row).transpose()?)
    }

    /// The user's password hash, for a password check by a signed-in user; `None` for an unknown
    /// id.
    pub async fn password_hash_by_id(&self, id: &str) -> Result<Option<String>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
                .bind(id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// Disable the user as of `at` (`Some`), or enable them (`None`). Disabling deletes their
    /// sessions and revokes their device tokens; enabling brings neither back. The last enabled
    /// admin cannot be disabled (`Conflict("last_admin")`). Disabling twice keeps the first time.
    pub async fn set_disabled(&self, id: &str, at: Option<i64>) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let row = sqlx::query("SELECT is_admin, disabled_at FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Err(StoreError::NotFound);
        };
        let is_admin: bool = row.try_get("is_admin")?;
        let disabled_at: Option<i64> = row.try_get("disabled_at")?;
        match at {
            Some(at) => {
                if is_admin && disabled_at.is_none() {
                    let others: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM users \
                         WHERE is_admin = 1 AND disabled_at IS NULL AND id != ?",
                    )
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
                    if others == 0 {
                        return Err(StoreError::Conflict("last_admin"));
                    }
                }
                sqlx::query("UPDATE users SET disabled_at = COALESCE(disabled_at, ?) WHERE id = ?")
                    .bind(at)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                revoke_credentials_in(&mut tx, id, at).await?;
            }
            None => {
                sqlx::query("UPDATE users SET disabled_at = NULL WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Replace the password hash, then delete every session, revoke every device token, deny the
    /// approved unpolled device codes and mark the unused reset codes used, all for this user. A
    /// caller keeping the current browser signed in mints a new session afterwards.
    pub async fn set_password(&self, id: &str, hash: &str, now: i64) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let done = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(hash)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        revoke_credentials_in(&mut tx, id, now).await?;
        invalidate_resets_in(&mut tx, id, now).await?;
        tx.commit().await?;
        Ok(())
    }

    // Sessions.

    /// A new 30-day session; returns the raw cookie value.
    pub async fn create_session(&self, user_id: &str, now: i64) -> Result<String, StoreError> {
        let raw = new_token();
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, expires_at, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(hash_token(&raw))
        .bind(user_id)
        .bind(now + SESSION_TTL_SECS)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(raw)
    }

    /// The user behind a session cookie; `None` when it is unknown, expired, or the user is
    /// disabled.
    pub async fn session_user(&self, raw: &str, now: i64) -> Result<Option<User>, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            user_columns!(),
            " FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = ? AND s.expires_at > ? AND u.disabled_at IS NULL"
        ))
        .bind(hash_token(raw))
        .bind(now)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.as_ref().map(user_from_row).transpose()?)
    }

    /// Sign one session out. Deleting an unknown session is not an error.
    pub async fn delete_session(&self, raw: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(hash_token(raw))
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Sign the user out everywhere but `keep_raw`; returns how many sessions went.
    pub async fn delete_sessions_except(
        &self,
        user_id: &str,
        keep_raw: &str,
    ) -> Result<u64, StoreError> {
        let done = sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
            .bind(user_id)
            .bind(hash_token(keep_raw))
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected())
    }

    pub async fn purge_expired_sessions(&self, now: i64) -> Result<u64, StoreError> {
        let done = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(now)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected())
    }

    // Invites.

    /// A new 7-day invite. A user may hold [`MAX_ACTIVE_INVITES`] active ones
    /// (`Conflict("too_many_invites")` past that); used, revoked and expired ones do not count.
    pub async fn create_invite(
        &self,
        user_id: &str,
        now: i64,
    ) -> Result<CreatedInvite, StoreError> {
        let mut tx = self.begin().await?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM invites WHERE created_by = ? \
             AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?",
        )
        .bind(user_id)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if active >= MAX_ACTIVE_INVITES {
            return Err(StoreError::Conflict("too_many_invites"));
        }
        let invite = CreatedInvite {
            id: new_id(),
            code: new_token(),
            expires_at: now + INVITE_TTL_SECS,
        };
        sqlx::query(
            "INSERT INTO invites (id, code_hash, created_by, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&invite.id)
        .bind(hash_token(&invite.code))
        .bind(user_id)
        .bind(now)
        .bind(invite.expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(invite)
    }

    /// What the invite page shows for a raw code; `None` for an unknown one. An invite whose
    /// creator is disabled reads as revoked.
    pub async fn invite_by_code(
        &self,
        raw: &str,
        now: i64,
    ) -> Result<Option<InviteInfo>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        Ok(invite_in(&mut conn, raw, now).await?.map(
            |(id, _, status, created_by_name, expires_at)| InviteInfo {
                id,
                status,
                created_by_name,
                expires_at,
            },
        ))
    }

    /// The user's invites, newest first.
    pub async fn list_invites(&self, user_id: &str) -> Result<Vec<InviteSummary>, StoreError> {
        let rows = sqlx::query(
            "SELECT i.id, i.created_at, i.expires_at, i.used_at, i.revoked_at, \
             u.display_name AS used_by_name FROM invites i LEFT JOIN users u ON u.id = i.used_by \
             WHERE i.created_by = ? ORDER BY i.created_at DESC, i.rowid DESC",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| -> Result<InviteSummary, sqlx::Error> {
                Ok(InviteSummary {
                    id: row.try_get("id")?,
                    created_at: row.try_get("created_at")?,
                    expires_at: row.try_get("expires_at")?,
                    used_at: row.try_get("used_at")?,
                    used_by_name: row.try_get("used_by_name")?,
                    revoked_at: row.try_get("revoked_at")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Revoke one of the user's own invites. Someone else's or an unknown one is `NotFound`, a
    /// used one is `Conflict("invite_used")`, and revoking twice is fine.
    pub async fn revoke_invite(&self, user_id: &str, id: &str, now: i64) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let row =
            sqlx::query("SELECT used_at, revoked_at FROM invites WHERE id = ? AND created_by = ?")
                .bind(id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(row) = row else {
            return Err(StoreError::NotFound);
        };
        if row.try_get::<Option<i64>, _>("used_at")?.is_some() {
            return Err(StoreError::Conflict("invite_used"));
        }
        if row.try_get::<Option<i64>, _>("revoked_at")?.is_none() {
            sqlx::query("UPDATE invites SET revoked_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Create a user from an invite and mark it used, in one transaction. An unknown code is
    /// `NotFound`; a used, expired or revoked one (or one whose creator is disabled) is a
    /// `Conflict` named after its state; a taken email is `Conflict("email_taken")` and leaves
    /// the invite unused.
    pub async fn accept_invite(
        &self,
        raw: &str,
        new: NewUser,
        now: i64,
    ) -> Result<User, StoreError> {
        let mut tx = self.begin().await?;
        let Some((id, created_by, status, _, _)) = invite_in(&mut tx, raw, now).await? else {
            return Err(StoreError::NotFound);
        };
        if let Some(code) = status.conflict() {
            return Err(StoreError::Conflict(code));
        }
        let user = insert_user_in(&mut tx, new, false, Some(&created_by), now).await?;
        let done = sqlx::query(
            "UPDATE invites SET used_by = ?, used_at = ? WHERE id = ? AND used_at IS NULL",
        )
        .bind(&user.id)
        .bind(now)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::Conflict("invite_used"));
        }
        tx.commit().await?;
        Ok(user)
    }

    // Password resets.

    /// A 24-hour, single-use reset code for `user_id`, made by the admin `created_by`. Returns
    /// the raw code. An unknown user is `NotFound`.
    pub async fn create_reset(
        &self,
        user_id: &str,
        created_by: &str,
        now: i64,
    ) -> Result<String, StoreError> {
        let mut tx = self.begin().await?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(StoreError::NotFound);
        }
        let raw = new_token();
        sqlx::query(
            "INSERT INTO password_resets (code_hash, user_id, created_by, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(hash_token(&raw))
        .bind(user_id)
        .bind(created_by)
        .bind(now)
        .bind(now + RESET_TTL_SECS)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(raw)
    }

    /// Use a reset code: set the new password hash, delete every session and revoke every
    /// device token of the user, deny their approved unpolled device codes, and mark their other
    /// reset codes used. Returns the user id. An unknown, used or expired code is
    /// `NotFound` (the caller need not say which).
    pub async fn consume_reset(
        &self,
        raw: &str,
        new_hash: &str,
        now: i64,
    ) -> Result<String, StoreError> {
        let mut tx = self.begin().await?;
        let user_id: Option<String> = sqlx::query_scalar(
            "UPDATE password_resets SET used_at = ? \
             WHERE code_hash = ? AND used_at IS NULL AND expires_at > ? RETURNING user_id",
        )
        .bind(now)
        .bind(hash_token(raw))
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(user_id) = user_id else {
            return Err(StoreError::NotFound);
        };
        let done = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(new_hash)
            .bind(&user_id)
            .execute(&mut *tx)
            .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        revoke_credentials_in(&mut tx, &user_id, now).await?;
        invalidate_resets_in(&mut tx, &user_id, now).await?;
        tx.commit().await?;
        Ok(user_id)
    }

    // The device-code flow.

    /// Start a 10-minute device flow for a device called `name`. A user code that collides with
    /// a live one is drawn again.
    pub async fn start_device(&self, name: &str, now: i64) -> Result<StartedDevice, StoreError> {
        const ATTEMPTS: u32 = 5;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let started = StartedDevice {
                device_code: new_token(),
                user_code: new_user_code(),
                expires_at: now + DEVICE_CODE_TTL_SECS,
            };
            let inserted = sqlx::query(
                "INSERT INTO device_codes (id, user_code, device_code_hash, device_name, \
                 created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(&started.user_code)
            .bind(hash_token(&started.device_code))
            .bind(name)
            .bind(now)
            .bind(started.expires_at)
            .execute(self.pool())
            .await;
            match inserted {
                Ok(_) => return Ok(started),
                Err(e) if attempt < ATTEMPTS && is_unique_violation(&e) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// The device code a person typed, in any case and with or without spaces or the dash.
    pub async fn device_by_user_code(
        &self,
        user_code: &str,
    ) -> Result<Option<DeviceCode>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        device_code_by_user_code_in(&mut conn, user_code).await
    }

    /// Approve a pending device code for `user_id`. Unknown or expired is `NotFound`; already
    /// approved or denied is `Conflict("device_code_used")`.
    pub async fn approve_device(
        &self,
        user_code: &str,
        user_id: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        self.decide_device(user_code, Some(user_id), now).await
    }

    /// Deny a pending device code; the same errors as [`Store::approve_device`].
    pub async fn deny_device(&self, user_code: &str, now: i64) -> Result<(), StoreError> {
        self.decide_device(user_code, None, now).await
    }

    async fn decide_device(
        &self,
        user_code: &str,
        approve_for: Option<&str>,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let Some(code) = device_code_by_user_code_in(&mut tx, user_code).await? else {
            return Err(StoreError::NotFound);
        };
        if code.expires_at <= now {
            return Err(StoreError::NotFound);
        }
        if !code.is_pending(now) {
            return Err(StoreError::Conflict("device_code_used"));
        }
        match approve_for {
            Some(user_id) => {
                sqlx::query(
                    "UPDATE device_codes SET approved_user_id = ?, approved_at = ? WHERE id = ?",
                )
                .bind(user_id)
                .bind(now)
                .bind(&code.id)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query("UPDATE device_codes SET denied_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(&code.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// One poll by the device. Every poll is recorded; a pending code polled again within
    /// [`DEVICE_POLL_INTERVAL_SECS`] gets `SlowDown`. Once approved, the poll mints the device
    /// token in the same transaction that marks the code consumed, so it is issued exactly once.
    /// An approval whose user has since been disabled reads as denied. An unknown code is
    /// `NotFound`.
    pub async fn poll_device(&self, raw: &str, now: i64) -> Result<PollOutcome, StoreError> {
        let mut tx = self.begin().await?;
        let row = sqlx::query(
            "SELECT d.id, d.device_name, d.expires_at, d.last_polled_at, d.approved_user_id, \
             d.denied_at, d.consumed_at, u.disabled_at AS user_disabled_at \
             FROM device_codes d LEFT JOIN users u ON u.id = d.approved_user_id \
             WHERE d.device_code_hash = ?",
        )
        .bind(hash_token(raw))
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Err(StoreError::NotFound);
        };
        let id: String = row.try_get("id")?;
        sqlx::query("UPDATE device_codes SET last_polled_at = ? WHERE id = ?")
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        let approved_user_id: Option<String> = row.try_get("approved_user_id")?;
        let last_polled_at: Option<i64> = row.try_get("last_polled_at")?;
        let outcome = if row.try_get::<Option<i64>, _>("consumed_at")?.is_some() {
            PollOutcome::Consumed
        } else if row.try_get::<Option<i64>, _>("denied_at")?.is_some() {
            PollOutcome::Denied
        } else if row.try_get::<i64, _>("expires_at")? <= now {
            PollOutcome::Expired
        } else if let Some(user_id) = approved_user_id {
            if row.try_get::<Option<i64>, _>("user_disabled_at")?.is_some() {
                PollOutcome::Denied
            } else {
                let name: String = row.try_get("device_name")?;
                let raw_token = format!("{DEVICE_TOKEN_PREFIX}{}", new_token());
                let prefix: String = raw_token.chars().take(DISPLAY_PREFIX_CHARS).collect();
                sqlx::query(
                    "INSERT INTO device_tokens (id, user_id, name, token_hash, prefix, \
                     created_at) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(new_id())
                .bind(&user_id)
                .bind(&name)
                .bind(hash_token(&raw_token))
                .bind(&prefix)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                let done = sqlx::query(
                    "UPDATE device_codes SET consumed_at = ? WHERE id = ? AND consumed_at IS NULL",
                )
                .bind(now)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
                if done.rows_affected() != 1 {
                    return Err(StoreError::Conflict("device_code_used"));
                }
                PollOutcome::Issued { raw_token }
            }
        } else if last_polled_at.is_some_and(|t| now - t < DEVICE_POLL_INTERVAL_SECS) {
            PollOutcome::SlowDown
        } else {
            PollOutcome::Pending
        };
        tx.commit().await?;
        Ok(outcome)
    }

    // Device tokens.

    /// The user behind a device token, with the token's id; `None` when it is unknown or
    /// revoked, or the user is disabled. Records `last_used_at`, at most once per
    /// [`LAST_USED_RESOLUTION_SECS`].
    pub async fn token_user(&self, raw: &str, now: i64) -> Result<Option<TokenUser>, StoreError> {
        let row = sqlx::query(concat!(
            "SELECT t.id AS token_id, t.last_used_at, ",
            user_columns!(),
            " FROM device_tokens t JOIN users u ON u.id = t.user_id \
             WHERE t.token_hash = ? AND t.revoked_at IS NULL AND u.disabled_at IS NULL"
        ))
        .bind(hash_token(raw))
        .fetch_optional(self.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let last_used_at: Option<i64> = row.try_get("last_used_at")?;
        let token_id: String = row.try_get("token_id")?;
        if last_used_at.is_none_or(|t| now - t >= LAST_USED_RESOLUTION_SECS) {
            // Conditional, so concurrent requests write it once.
            sqlx::query(
                "UPDATE device_tokens SET last_used_at = ? \
                 WHERE id = ? AND (last_used_at IS NULL OR last_used_at <= ?)",
            )
            .bind(now)
            .bind(&token_id)
            .bind(now - LAST_USED_RESOLUTION_SECS)
            .execute(self.pool())
            .await?;
        }
        Ok(Some(TokenUser {
            token_id,
            user: user_from_row(&row)?,
        }))
    }

    /// The user's signed-in (unrevoked) devices, newest first.
    pub async fn list_devices(&self, user_id: &str) -> Result<Vec<DeviceToken>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, name, prefix, created_at, last_used_at FROM device_tokens \
             WHERE user_id = ? AND revoked_at IS NULL ORDER BY created_at DESC, rowid DESC",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| -> Result<DeviceToken, sqlx::Error> {
                Ok(DeviceToken {
                    id: row.try_get("id")?,
                    name: row.try_get("name")?,
                    prefix: row.try_get("prefix")?,
                    created_at: row.try_get("created_at")?,
                    last_used_at: row.try_get("last_used_at")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Revoke one of the user's own devices. Someone else's, an unknown or an already revoked
    /// one is `NotFound`.
    pub async fn revoke_device(&self, user_id: &str, id: &str, now: i64) -> Result<(), StoreError> {
        let done = sqlx::query(
            "UPDATE device_tokens SET revoked_at = ? \
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(id)
        .bind(user_id)
        .execute(self.pool())
        .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    // Owned matches.

    /// The candidates for the user's live matches: their newest [`OWNED_MATCH_CANDIDATES`]
    /// matches by last activity, newest first. Whether a match is finished takes the engine
    /// (replaying its log), which the store does not run, so the caller filters out the finished
    /// ones and keeps the first 20. A live match is one being scored, so it sits near the top.
    pub async fn live_matches_for_owner(
        &self,
        user_id: &str,
    ) -> Result<Vec<StoredMatch>, StoreError> {
        let rows = sqlx::query(select_matches!(
            "WHERE owner_user_id = ? ORDER BY last_activity_at DESC, created_at DESC, rowid DESC \
             LIMIT ?"
        ))
        .bind(user_id)
        .bind(OWNED_MATCH_CANDIDATES)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(match_from_row)
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// The match, if `user_id` owns it.
    pub async fn match_owned_by(
        &self,
        id: &str,
        user_id: &str,
    ) -> Result<Option<StoredMatch>, StoreError> {
        let row = sqlx::query(select_matches!("WHERE id = ? AND owner_user_id = ?"))
            .bind(id)
            .bind(user_id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(match_from_row).transpose()?)
    }

    // Retention.

    /// Delete expired sessions, invites, device codes and reset codes, in one transaction.
    pub async fn purge_expired_auth(&self, now: i64) -> Result<PurgedAuth, StoreError> {
        let mut tx = self.begin().await?;
        let mut purge = async |sql: &'static str| -> Result<u64, StoreError> {
            Ok(sqlx::query(sql)
                .bind(now)
                .execute(&mut *tx)
                .await?
                .rows_affected())
        };
        let purged = PurgedAuth {
            sessions: purge("DELETE FROM sessions WHERE expires_at <= ?").await?,
            invites: purge("DELETE FROM invites WHERE expires_at <= ?").await?,
            device_codes: purge("DELETE FROM device_codes WHERE expires_at <= ?").await?,
            resets: purge("DELETE FROM password_resets WHERE expires_at <= ?").await?,
        };
        tx.commit().await?;
        Ok(purged)
    }
}
