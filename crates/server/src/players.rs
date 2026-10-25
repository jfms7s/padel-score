//! The coordinator's player list: named people a signed-in user picks from when setting up a
//! match. Players are archived, never deleted, and unique per owner on [`name_key`].

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqliteConnection};

use crate::store::{MAX_NAME_CHARS, Store, StoreError, Teams};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct PlayerChange {
    pub name: Option<String>,
    pub archived: Option<bool>,
}

/// The players picked from the list for a match's four slots; `None` is a free-text slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSlots {
    #[serde(default)]
    pub a: [Option<String>; 2],
    #[serde(default)]
    pub b: [Option<String>; 2],
}

impl PlayerSlots {
    fn is_empty(&self) -> bool {
        self.a.iter().chain(&self.b).all(Option::is_none)
    }
}

/// Check the picked players for a new match owned by `owner` and put each one's current name in
/// its slot of `teams`. Returns `(team, position, player id)` for every picked slot.
pub(crate) async fn resolve_slots_in(
    conn: &mut SqliteConnection,
    owner: Option<&str>,
    slots: &PlayerSlots,
    teams: &mut Teams,
) -> Result<Vec<(&'static str, i64, String)>, StoreError> {
    if slots.is_empty() {
        return Ok(Vec::new());
    }
    let Some(owner) = owner else {
        return Err(StoreError::InvalidPlayers("players need an account"));
    };
    let mut picked = Vec::new();
    let sides = [("a", &slots.a, &mut teams.a), ("b", &slots.b, &mut teams.b)];
    for (team, ids, names) in sides {
        for (position, (id, name)) in ids.iter().zip(names.iter_mut()).enumerate() {
            let Some(id) = id else { continue };
            let player = player_in(conn, owner, id)
                .await?
                .filter(|p| !p.archived)
                .ok_or(StoreError::InvalidPlayers("unknown player"))?;
            if picked.iter().any(|(_, _, other)| *other == player.id) {
                return Err(StoreError::InvalidPlayers("same player twice"));
            }
            *name = player.name;
            picked.push((team, position as i64, player.id));
        }
    }
    Ok(picked)
}

/// Trim a player name; blank or over 40 chars (not bytes) is `InvalidPlayerName`.
pub fn normalize_name(raw: &str) -> Result<String, StoreError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::InvalidPlayerName);
    }
    Ok(name.to_owned())
}

/// The uniqueness key of a name: trimmed and Unicode-lowercased.
pub fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// `SELECT <every players column> FROM players <rest>`, as a `&'static str`.
macro_rules! select_players {
    ($rest:literal) => {
        concat!(
            "SELECT id, name, archived_at, created_at FROM players ",
            $rest
        )
    };
}

fn player_from_row(row: &SqliteRow) -> Result<Player, sqlx::Error> {
    Ok(Player {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        archived: row.try_get::<Option<i64>, _>("archived_at")?.is_some(),
        created_at: row.try_get("created_at")?,
    })
}

pub(crate) async fn player_in(
    conn: &mut SqliteConnection,
    owner: &str,
    id: &str,
) -> Result<Option<Player>, StoreError> {
    let row = sqlx::query(select_players!("WHERE owner_user_id = ? AND id = ?"))
        .bind(owner)
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(player_from_row).transpose()?)
}

async fn player_by_key_in(
    conn: &mut SqliteConnection,
    owner: &str,
    key: &str,
) -> Result<Option<Player>, StoreError> {
    let row = sqlx::query(select_players!("WHERE owner_user_id = ? AND name_key = ?"))
        .bind(owner)
        .bind(key)
        .fetch_optional(conn)
        .await?;
    Ok(row.as_ref().map(player_from_row).transpose()?)
}

impl Store {
    /// The owner's player named `raw_name`, creating it if there is none. The flag is `true` when
    /// created. An existing player (matched on [`name_key`]) is returned as is, and un-archived
    /// if it was archived.
    pub async fn create_or_get_player(
        &self,
        owner: &str,
        raw_name: &str,
        now: i64,
    ) -> Result<(Player, bool), StoreError> {
        let name = normalize_name(raw_name)?;
        let key = name_key(&name);
        let mut tx = self.begin().await?;
        if let Some(mut existing) = player_by_key_in(&mut tx, owner, &key).await? {
            if existing.archived {
                sqlx::query("UPDATE players SET archived_at = NULL WHERE id = ?")
                    .bind(&existing.id)
                    .execute(&mut *tx)
                    .await?;
                existing.archived = false;
            }
            tx.commit().await?;
            return Ok((existing, false));
        }
        let player = Player {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            archived: false,
            created_at: now,
        };
        sqlx::query(
            "INSERT INTO players (id, owner_user_id, name, name_key, archived_at, created_at) \
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&player.id)
        .bind(owner)
        .bind(&player.name)
        .bind(&key)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((player, true))
    }

    /// Every player of the owner, archived included, ordered by name key.
    pub async fn list_players(&self, owner: &str) -> Result<Vec<Player>, StoreError> {
        let rows = sqlx::query(select_players!(
            "WHERE owner_user_id = ? ORDER BY name_key, id"
        ))
        .bind(owner)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(player_from_row)
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub async fn player(&self, owner: &str, id: &str) -> Result<Option<Player>, StoreError> {
        let mut conn = self.pool().acquire().await?;
        player_in(&mut conn, owner, id).await
    }

    /// Rename and/or archive. An unknown id, or another owner's, is `NotFound`; a rename onto
    /// another player's name key is `Conflict("player_exists")`. A case-only rename is allowed.
    /// The archive time is the database clock (`unixepoch()`): the signature carries no `now`,
    /// and only whether it is set is ever read.
    pub async fn update_player(
        &self,
        owner: &str,
        id: &str,
        change: PlayerChange,
    ) -> Result<Player, StoreError> {
        let new_name = change.name.as_deref().map(normalize_name).transpose()?;
        let mut tx = self.begin().await?;
        let mut player = player_in(&mut tx, owner, id)
            .await?
            .ok_or(StoreError::NotFound)?;
        if let Some(name) = new_name {
            let key = name_key(&name);
            if let Some(other) = player_by_key_in(&mut tx, owner, &key).await?
                && other.id != player.id
            {
                return Err(StoreError::Conflict("player_exists"));
            }
            sqlx::query("UPDATE players SET name = ?, name_key = ? WHERE id = ?")
                .bind(&name)
                .bind(&key)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            player.name = name;
        }
        if let Some(archived) = change.archived
            && archived != player.archived
        {
            sqlx::query(
                "UPDATE players SET archived_at = CASE WHEN ? THEN unixepoch() ELSE NULL END \
                 WHERE id = ?",
            )
            .bind(archived)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            player.archived = archived;
        }
        tx.commit().await?;
        Ok(player)
    }
}
