//! The board side of the calendar: the user's games, the requests waiting for their answer, their
//! boards that are not over, and the short-game reminder.

use std::collections::HashMap;

use sqlx::SqliteConnection;

use super::{BoardTuple, RequestKind, game_state, view_in};
use crate::calendar::{BoardStrip, CalendarItem, MAX_ITEMS, WaitingItem};
use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{StoreError, Tx};

/// The user's games starting at or after `from`, on any board they have a seat in (a board they
/// left has none). Each comes with its id for sorting.
pub(crate) async fn calendar_games_in(
    conn: &mut SqliteConnection,
    user: &str,
    from: i64,
    now: i64,
) -> Result<Vec<(String, CalendarItem)>, StoreError> {
    let games: Vec<(String, i64, i64, bool, String, String)> = sqlx::query_as(
        "SELECT g.id, g.starts_at, g.ends_at, g.match_id IS NOT NULL, b.token, b.title \
         FROM board_game_players p JOIN board_games g ON g.id = p.game_id \
         JOIN boards b ON b.id = g.board_id \
         WHERE p.user_id = ? AND g.starts_at >= ? ORDER BY g.starts_at, g.id LIMIT ?",
    )
    .bind(user)
    .bind(from)
    .bind(i64::try_from(MAX_ITEMS).unwrap_or(i64::MAX))
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (id, start, end, played, board_token, board_title) in games {
        let players: Vec<String> = sqlx::query_scalar(
            "SELECT u.display_name FROM board_game_players p JOIN users u ON u.id = p.user_id \
             WHERE p.game_id = ? ORDER BY p.seat",
        )
        .bind(&id)
        .fetch_all(&mut *conn)
        .await?;
        let state = game_state(played, end, players.len(), now);
        out.push((
            id.clone(),
            CalendarItem::Game {
                board_token,
                board_title,
                game_id: id,
                start,
                end,
                players,
                state,
            },
        ));
    }
    Ok(out)
}

/// For the boards the user is on that are not over: the requests waiting for their answer (stale
/// ones left out, as the board page does) and a strip per board.
pub(crate) async fn waiting_and_strips_in(
    conn: &mut SqliteConnection,
    user: &str,
    now: i64,
) -> Result<(Vec<WaitingItem>, Vec<BoardStrip>), StoreError> {
    let boards: Vec<BoardTuple> = sqlx::query_as(select_board!(
        "WHERE b.ends_at > ? AND b.id IN \
         (SELECT board_id FROM board_members WHERE user_id = ?) \
         ORDER BY b.starts_at, b.id"
    ))
    .bind(now)
    .bind(user)
    .fetch_all(&mut *conn)
    .await?;
    let mut waiting = Vec::new();
    let mut strips = Vec::new();
    for board in boards.into_iter().map(super::BoardRow::from_tuple) {
        let view = view_in(conn, &board, Some(user), now).await?;
        let times: HashMap<&str, (i64, i64)> = view
            .games
            .iter()
            .map(|g| (g.id.as_str(), (g.start, g.end)))
            .collect();
        for request in view.requests.iter().filter(|r| r.needs_my_answer) {
            let Some(&theirs) = times.get(request.game_id.as_str()) else {
                continue;
            };
            let from_name = request.from.name.clone();
            let board_token = board.token.clone();
            let request_id = request.id.clone();
            match request.kind {
                RequestKind::Swap => {
                    // The asked player holds the other game.
                    let Some(&mine) = request
                        .other_game_id
                        .as_deref()
                        .and_then(|id| times.get(id))
                    else {
                        continue;
                    };
                    waiting.push(WaitingItem::Swap {
                        request_id,
                        board_token,
                        from_name,
                        my_game: mine,
                        their_game: theirs,
                    });
                }
                RequestKind::Time => {
                    let Some(new) = request.new_start.zip(request.new_end) else {
                        continue;
                    };
                    waiting.push(WaitingItem::Time {
                        request_id,
                        board_token,
                        from_name,
                        game: theirs,
                        new,
                    });
                }
            }
        }
        let my_games = view
            .games
            .iter()
            .filter(|g| g.end > now && g.players.iter().any(|p| p.user_id == user))
            .count();
        strips.push(BoardStrip {
            token: board.token.clone(),
            title: board.title.clone(),
            starts_at: board.starts_at,
            ends_at: board.ends_at,
            my_games,
        });
    }
    Ok((waiting, strips))
}

/// `short_game` for games with three players that start within the next day, to every member of
/// the board who is in no game overlapping it. Returns how many notices were queued.
pub(crate) async fn queue_short_games_in(tx: &mut Tx<'_>, now: i64) -> Result<usize, StoreError> {
    let games: Vec<(String, String, i64, i64, String, String)> = sqlx::query_as(
        "SELECT g.id, g.board_id, g.starts_at, g.ends_at, b.token, b.title \
         FROM board_games g JOIN boards b ON b.id = g.board_id \
         WHERE g.match_id IS NULL AND g.starts_at > ?1 AND g.starts_at <= ?1 + ?2 \
         AND (SELECT COUNT(*) FROM board_game_players p WHERE p.game_id = g.id) = 3 \
         ORDER BY g.starts_at, g.id",
    )
    .bind(now)
    .bind(crate::calendar::DAY_SECS)
    .fetch_all(&mut **tx)
    .await?;
    let mut queued = 0;
    for (game_id, board_id, start, end, token, title) in games {
        let free: Vec<String> = sqlx::query_scalar(
            "SELECT m.user_id FROM board_members m WHERE m.board_id = ?1 AND NOT EXISTS ( \
               SELECT 1 FROM board_game_players p JOIN board_games o ON o.id = p.game_id \
               WHERE p.user_id = m.user_id AND o.starts_at < ?3 AND o.ends_at > ?2) \
             ORDER BY m.user_id",
        )
        .bind(&board_id)
        .bind(start)
        .bind(end)
        .fetch_all(&mut **tx)
        .await?;
        let notices: Vec<Notice> = free
            .into_iter()
            .map(|user| Notice {
                dedupe: Some(format!("short:{game_id}:{user}")),
                user_id: user,
                payload: Payload {
                    kind: NoticeKind::ShortGame,
                    url: format!("/b/{token}"),
                    title: Some(title.clone()),
                    start: Some(start),
                    end: Some(end),
                    ..Payload::default()
                },
            })
            .collect();
        queued += queue_in(tx, &notices, now).await?;
    }
    Ok(queued)
}
