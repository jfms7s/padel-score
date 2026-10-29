//! The match side of the Organise page: the scheduled matches a user owns.

use sqlx::SqliteConnection;

use super::{filled_in, state_of};
use crate::organised::{MAX_MATCHES, OrganisedMatch};
use crate::store::StoreError;

/// `(id, join token, starts at, place, cancelled, started)`.
type MatchRow = (String, String, i64, Option<String>, bool, bool);

/// The latest [`MAX_MATCHES`] matches `user` owns, by start descending, with who holds each
/// place and how many wait in line.
pub(crate) async fn organised_matches_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<Vec<OrganisedMatch>, StoreError> {
    let rows: Vec<MatchRow> = sqlx::query_as(
        "SELECT id, join_token, starts_at, place, cancelled_at IS NOT NULL, \
         started_at IS NOT NULL FROM scheduled_matches WHERE owner_user_id = ? \
         ORDER BY starts_at DESC, rowid DESC LIMIT ?",
    )
    .bind(user)
    .bind(i64::try_from(MAX_MATCHES).unwrap_or(i64::MAX))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (id, join_token, starts_at, place, cancelled, started) in rows {
        let filled = filled_in(conn, &id).await?;
        let line: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_line WHERE scheduled_id = ?")
                .bind(&id)
                .fetch_one(&mut *conn)
                .await?;
        let mut places: [Option<String>; 4] = Default::default();
        for f in &filled {
            if let Some(cell) = places.get_mut(usize::from(f.slot)) {
                *cell = Some(f.name.clone());
            }
        }
        out.push(OrganisedMatch {
            id,
            join_token,
            starts_at,
            place,
            state: state_of(cancelled, started, starts_at, filled.len(), now),
            places,
            line_count: usize::try_from(line).unwrap_or(0),
        });
    }
    Ok(out)
}
