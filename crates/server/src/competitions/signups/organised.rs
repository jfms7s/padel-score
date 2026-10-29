//! The event side of the Organise page: the events a user owns, with their sign-up counts.

use sqlx::SqliteConnection;

use super::layout_in;
use crate::competitions::competition_by_id_in;
use crate::competitions::view::EventSummary;
use crate::organised::OrganisedEvent;
use crate::store::StoreError;

/// Every event `user` owns, newest first, each with its team and court counts as
/// `GET /api/events` lists them and its sign-up counts.
pub(crate) async fn organised_events_in(
    conn: &mut SqliteConnection,
    user: &str,
) -> Result<Vec<OrganisedEvent>, StoreError> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT c.id, \
           (SELECT COUNT(*) FROM competition_teams WHERE event_id = c.id), \
           (SELECT COUNT(*) FROM courts WHERE event_id = c.id) \
         FROM competitions c WHERE c.owner_user_id = ? ORDER BY c.created_at DESC, c.rowid DESC",
    )
    .bind(user)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (id, teams, courts) in rows {
        let Some(ev) = competition_by_id_in(conn, &id).await? else {
            continue;
        };
        let layout = layout_in(conn, &ev.id, ev.schedule.max_teams).await?;
        out.push(OrganisedEvent {
            summary: EventSummary::new(
                &ev,
                usize::try_from(teams).unwrap_or(0),
                usize::try_from(courts).unwrap_or(0),
            ),
            join_token: ev.join_token.clone(),
            signed_teams: layout.team_count(),
            solo_count: layout.solo_count(),
            line_count: layout.line_count(),
        });
    }
    Ok(out)
}
