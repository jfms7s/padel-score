//! The board side of the Organise page: the boards a user created.

use sqlx::SqliteConnection;

use crate::organised::{MAX_BOARDS, OrganisedBoard};
use crate::store::StoreError;

/// `(token, title, place, starts at, ends at, members, games)`.
type BoardRow = (String, String, Option<String>, i64, i64, i64, i64);

/// The latest [`MAX_BOARDS`] boards `user` created, by start descending, whether or not they are
/// still a member. `over` is `ends_at <= now`; `member_count` counts everyone on the board now,
/// guests included (a guest who is gone has no row).
pub(crate) async fn organised_boards_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<Vec<OrganisedBoard>, StoreError> {
    let rows: Vec<BoardRow> = sqlx::query_as(
        "SELECT b.token, b.title, b.place, b.starts_at, b.ends_at, \
           (SELECT COUNT(*) FROM board_members WHERE board_id = b.id), \
           (SELECT COUNT(*) FROM board_games WHERE board_id = b.id) \
         FROM boards b WHERE b.creator_user_id = ? ORDER BY b.starts_at DESC, b.id LIMIT ?",
    )
    .bind(user)
    .bind(i64::try_from(MAX_BOARDS).unwrap_or(i64::MAX))
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(token, title, place, starts_at, ends_at, members, games)| OrganisedBoard {
                token,
                title,
                place,
                starts_at,
                ends_at,
                over: ends_at <= now,
                member_count: usize::try_from(members).unwrap_or(0),
                game_count: usize::try_from(games).unwrap_or(0),
            },
        )
        .collect())
}
