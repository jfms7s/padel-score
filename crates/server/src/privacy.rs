//! Who may see a player's combined history. [`viewer_of`] is the only rule: routes call it and
//! never decide for themselves. [`redact`] then strips what a viewer may not see of each match.
//!
//! The setting lives in `users.history_visibility`. A public profile link is
//! `users.profile_token`, set while the visibility is `public` and cleared when it moves away, so
//! leaving `public` kills the link at once; going public again makes a new one.

use serde::{Deserialize, Serialize};

use crate::accounts_store::User;
use crate::linked_history::{LinkedEntry, LinkedPlacing};
use crate::store::{Store, StoreError};
use crate::tokens::new_token;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Private,
    Coordinators,
    Public,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Coordinators => "coordinators",
            Visibility::Public => "public",
        }
    }

    pub fn parse(text: &str) -> Option<Visibility> {
        match text {
            "private" => Some(Visibility::Private),
            "coordinators" => Some(Visibility::Coordinators),
            "public" => Some(Visibility::Public),
            _ => None,
        }
    }
}

/// Who is looking, once allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    /// The player themself.
    Own,
    /// A coordinator who owns an entry linked to the player.
    Coordinator { id: String },
    /// Anyone holding the public profile link.
    Public,
}

/// Whether and as whom `viewer_user_id` (signed in, if at all; `via_profile_token` when they
/// came by a profile link) sees `subject_user_id`'s combined history:
///
/// - the subject themself: [`Viewer::Own`];
/// - `coordinators` or `public`, and the viewer owns an entry linked to the subject:
///   [`Viewer::Coordinator`];
/// - `public` and a profile link: [`Viewer::Public`];
/// - anything else: `None`.
pub async fn viewer_of(
    store: &Store,
    subject_user_id: &str,
    viewer_user_id: Option<&str>,
    via_profile_token: bool,
) -> Result<Option<Viewer>, StoreError> {
    if viewer_user_id == Some(subject_user_id) {
        return Ok(Some(Viewer::Own));
    }
    let (visibility, _) = store.visibility(subject_user_id).await?;
    if visibility == Visibility::Private {
        return Ok(None);
    }
    if let Some(viewer) = viewer_user_id
        && store.coordinates(viewer, subject_user_id).await?
    {
        return Ok(Some(Viewer::Coordinator {
            id: viewer.to_owned(),
        }));
    }
    if visibility == Visibility::Public && via_profile_token {
        return Ok(Some(Viewer::Public));
    }
    Ok(None)
}

/// Whether `viewer` sees in full what `owner_user_id` coordinated: the player sees all of their
/// own, and a coordinator all of their own matches and events.
fn sees_all(viewer: &Viewer, owner_user_id: &str) -> bool {
    match viewer {
        Viewer::Own => true,
        Viewer::Coordinator { id } => id == owner_user_id,
        Viewer::Public => false,
    }
}

/// Clear what `viewer` may not see of `entry`: the teams, the coordinator's name, the match
/// link, and all of an event match's event but its kind.
pub fn redact(mut entry: LinkedEntry, viewer: &Viewer) -> LinkedEntry {
    if !sees_all(viewer, &entry.owner_user_id) {
        entry.teams = None;
        entry.coordinator_name = None;
        entry.spectator_token = None;
        entry.event = entry.event.map(|e| e.redacted());
        entry.redacted = true;
    }
    entry
}

/// Clear the event's name from a placing `viewer` may not see in full.
pub fn redact_placing(mut placing: LinkedPlacing, viewer: &Viewer) -> LinkedPlacing {
    if !sees_all(viewer, &placing.owner_user_id) {
        placing.event_name = None;
    }
    placing
}

impl Store {
    /// Whether `coordinator` owns at least one entry linked to `user_id`.
    async fn coordinates(&self, coordinator: &str, user_id: &str) -> Result<bool, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM players \
             WHERE owner_user_id = ? AND linked_user_id = ?)",
        )
        .bind(coordinator)
        .bind(user_id)
        .fetch_one(self.pool())
        .await?)
    }

    /// The user's setting and public profile token (set only while `public`). An unknown user is
    /// `NotFound`.
    pub async fn visibility(
        &self,
        user_id: &str,
    ) -> Result<(Visibility, Option<String>), StoreError> {
        let row: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT history_visibility, profile_token FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_optional(self.pool())
                .await?;
        let (text, token) = row.ok_or(StoreError::NotFound)?;
        let visibility = Visibility::parse(&text)
            .ok_or_else(|| StoreError::Corrupt(format!("unknown visibility {text:?}")))?;
        Ok((visibility, token))
    }

    /// Change the setting and return the profile token. Moving to `public` makes a token if there
    /// is none; moving away clears it, so the old link dies.
    pub async fn set_visibility(
        &self,
        user_id: &str,
        visibility: Visibility,
    ) -> Result<Option<String>, StoreError> {
        let token = (visibility == Visibility::Public).then(new_token);
        let done = sqlx::query(
            "UPDATE users SET history_visibility = ?1, \
             profile_token = CASE WHEN ?1 = 'public' THEN COALESCE(profile_token, ?2) END \
             WHERE id = ?3",
        )
        .bind(visibility.as_str())
        .bind(token)
        .bind(user_id)
        .execute(self.pool())
        .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::NotFound);
        }
        Ok(self.visibility(user_id).await?.1)
    }

    /// Replace the profile token, so the old link stops working. `Conflict("not_public")` unless
    /// the setting is `public`.
    pub async fn rotate_profile_token(&self, user_id: &str) -> Result<String, StoreError> {
        let token = new_token();
        let done = sqlx::query(
            "UPDATE users SET profile_token = ? WHERE id = ? AND history_visibility = 'public'",
        )
        .bind(&token)
        .bind(user_id)
        .execute(self.pool())
        .await?;
        if done.rows_affected() != 1 {
            return Err(StoreError::Conflict("not_public"));
        }
        Ok(token)
    }

    /// The user whose public profile link is `token`; `None` for an unknown token, a user who is
    /// not `public`, or a disabled user.
    pub async fn user_by_profile_token(&self, token: &str) -> Result<Option<User>, StoreError> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM users WHERE profile_token = ? AND history_visibility = 'public' \
             AND disabled_at IS NULL",
        )
        .bind(token)
        .fetch_optional(self.pool())
        .await?;
        match id {
            Some(id) => self.user_by_id(&id).await,
            None => Ok(None),
        }
    }
}
