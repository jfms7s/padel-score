//! What a player does to one game: leave it, give their place away, take an open place, swap
//! places with a player of an overlapping game or change its time. Every
//! action carries the game's `version` from the view the player saw, and a mismatch is
//! `game_changed`, so a stale sheet never moves anyone. Each is one `BEGIN IMMEDIATE`
//! transaction that ends in [`settle_in`].

use std::collections::HashMap;

use sqlx::SqliteConnection;

use super::formation::{self, BoardState, Give, GiveRefusal, PLACES, can_swap, check_new_time};
use super::ranges::{Trim, trim, trim_ranges, uncovered_parts};
use super::{
    BoardRow, BoardView, MAX_PIECES_PER_MEMBER, MemberRef, RequestKind, RequestOut,
    display_name_in, find_by_id_in, find_by_token_in, insert_piece_in, join_in, load_state_in,
    mine, settle_in, view_in,
};
use crate::push::{Notice, NoticeKind, queue_in};
use crate::store::{Store, StoreError, Tx};

/// The columns of a `board_games` row the actions need.
pub(super) struct GameRow {
    pub(super) start: i64,
    pub(super) end: i64,
    pub(super) version: i64,
    pub(super) played: bool,
}

async fn game_row_in(
    conn: &mut SqliteConnection,
    board_id: &str,
    game_id: &str,
) -> Result<GameRow, StoreError> {
    let row: Option<(i64, i64, i64, bool)> = sqlx::query_as(
        "SELECT starts_at, ends_at, version, match_id IS NOT NULL FROM board_games \
         WHERE id = ? AND board_id = ?",
    )
    .bind(game_id)
    .bind(board_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (start, end, version, played) = row.ok_or(StoreError::NotFound)?;
    Ok(GameRow {
        start,
        end,
        version,
        played,
    })
}

/// The board and game for an action, checked in this order: the token and the game on that
/// board are `NotFound`, then `board_over`, then a stale `version` is `game_changed`.
async fn open_game_in(
    conn: &mut SqliteConnection,
    token: &str,
    game_id: &str,
    version: i64,
    now: i64,
) -> Result<(BoardRow, GameRow), StoreError> {
    let board = find_by_token_in(conn, token)
        .await?
        .ok_or(StoreError::NotFound)?;
    let game = game_row_in(conn, &board.id, game_id).await?;
    if board.over(now) {
        return Err(StoreError::Conflict("board_over"));
    }
    if game.version != version {
        return Err(StoreError::Conflict("game_changed"));
    }
    Ok((board, game))
}

/// A played game, or one that has started, takes no more actions.
fn still_open(game: &GameRow, now: i64) -> Result<(), StoreError> {
    if game.played {
        return Err(StoreError::Conflict("game_played"));
    }
    if game.start <= now {
        return Err(StoreError::Conflict("game_started"));
    }
    Ok(())
}

async fn bump_version_in(tx: &mut Tx<'_>, game_id: &str) -> Result<(), StoreError> {
    sqlx::query("UPDATE board_games SET version = version + 1 WHERE id = ?")
        .bind(game_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn drop_line_head_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM board_line_heads WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn seat_of_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<i64, StoreError> {
    let seat: Option<i64> =
        sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ? AND user_id = ?")
            .bind(game_id)
            .bind(user)
            .fetch_optional(&mut **tx)
            .await?;
    seat.ok_or_else(|| StoreError::Corrupt(format!("{user} has no seat in game {game_id}")))
}

async fn seat_in(
    tx: &mut Tx<'_>,
    game_id: &str,
    user: &str,
    seat: i64,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) VALUES (?, ?, ?, ?)",
    )
    .bind(game_id)
    .bind(user)
    .bind(seat)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn unseat_in(tx: &mut Tx<'_>, game_id: &str, user: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM board_game_players WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Write a [`Trim`]: delete the cut pieces and insert what is left of them.
async fn apply_trim_in(
    tx: &mut Tx<'_>,
    board_id: &str,
    user: &str,
    cut: Trim,
) -> Result<(), StoreError> {
    for id in &cut.delete {
        sqlx::query("DELETE FROM board_ranges WHERE id = ?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    for (start, end, created_at) in cut.insert {
        insert_piece_in(tx, board_id, user, start, end, created_at).await?;
    }
    Ok(())
}

/// Trade two players' seats between two games, each taking over the other's seat number. Both
/// games' versions go up and neither player heads either line any more.
async fn trade_seats_in(
    tx: &mut Tx<'_>,
    game_a: &str,
    user_a: &str,
    game_b: &str,
    user_b: &str,
    now: i64,
) -> Result<(), StoreError> {
    let seat_a = seat_of_in(tx, game_a, user_a).await?;
    let seat_b = seat_of_in(tx, game_b, user_b).await?;
    unseat_in(tx, game_a, user_a).await?;
    unseat_in(tx, game_b, user_b).await?;
    seat_in(tx, game_b, user_a, seat_b, now).await?;
    seat_in(tx, game_a, user_b, seat_a, now).await?;
    for (game, user) in [
        (game_a, user_a),
        (game_a, user_b),
        (game_b, user_a),
        (game_b, user_b),
    ] {
        drop_line_head_in(tx, game, user).await?;
    }
    bump_version_in(tx, game_a).await?;
    bump_version_in(tx, game_b).await
}

/// Take `user` out of a game. An emptied game is deleted; otherwise its version goes up and the
/// players left are told. The leaver stops heading the game's line. It does not fill the seat:
/// [`settle_in`] does, after the caller's changes are all made.
pub(crate) async fn leave_in(
    tx: &mut Tx<'_>,
    board_id: &str,
    game_id: &str,
    user: &str,
    now: i64,
) -> Result<(), StoreError> {
    drop_line_head_in(tx, game_id, user).await?;
    let removed = sqlx::query("DELETE FROM board_game_players WHERE game_id = ? AND user_id = ?")
        .bind(game_id)
        .bind(user)
        .execute(&mut **tx)
        .await?;
    if removed.rows_affected() == 0 {
        return Ok(());
    }
    let rest: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM board_game_players WHERE game_id = ? ORDER BY seat",
    )
    .bind(game_id)
    .fetch_all(&mut **tx)
    .await?;
    if rest.is_empty() {
        sqlx::query("DELETE FROM board_games WHERE id = ?")
            .bind(game_id)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }
    bump_version_in(tx, game_id).await?;
    let game = game_row_in(tx, board_id, game_id).await?;
    let board = find_by_id_in(tx, board_id).await?;
    let name = display_name_in(tx, user).await?;
    let notices: Vec<Notice> = rest
        .iter()
        .map(|other| {
            board.notice(
                other,
                NoticeKind::PlayerLeft,
                board.board_url(),
                (game.start, game.end),
                Some(name.clone()),
            )
        })
        .collect();
    queue_in(tx, &notices, now).await?;
    Ok(())
}

impl Store {
    /// Leave a game. The leaver's free time over the game goes too, or the next settle would
    /// seat them straight back. The first in the line takes the place, unless the game starts
    /// inside the fill cutoff: then it stays short.
    pub async fn leave_game(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let seated = state
            .games
            .iter()
            .any(|g| g.id == game_id && g.players.iter().any(|p| p == user));
        if !seated {
            return Err(StoreError::Conflict("not_in_game"));
        }
        // Leave is never refused for the piece cap: the game's time comes out of a long piece
        // even when that makes two.
        let cut = trim(&mine(&state, user), game.start, game.end);
        apply_trim_in(&mut tx, &board.id, user, cut).await?;
        leave_in(&mut tx, &board.id, game_id, user, now).await?;
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Give the player's place away: to the next overlapping game, trading seats with its best
    /// free player, or else to the front of the line. See [`formation::give_place`].
    pub async fn give_place(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let give = formation::give_place(&state, game_id, user, now).map_err(|refusal| {
            StoreError::Conflict(match refusal {
                GiveRefusal::NotInGame => "not_in_game",
                GiveRefusal::TooLate => "too_late",
                GiveRefusal::NobodyCanTake => "nobody_can_take",
            })
        })?;
        let name = display_name_in(&mut tx, user).await?;
        match give {
            Give::Trade {
                other_game,
                other_user,
            } => {
                let other = game_row_in(&mut tx, &board.id, &other_game).await?;
                trade_seats_in(&mut tx, game_id, user, &other_game, &other_user, now).await?;
                let mut notice = board.notice(
                    &other_user,
                    NoticeKind::PlaceGiven,
                    board.board_url(),
                    (other.start, other.end),
                    Some(name),
                );
                notice.payload.new_start = Some(game.start);
                notice.payload.new_end = Some(game.end);
                queue_in(&mut tx, &[notice], now).await?;
            }
            Give::ToLine { replacement } => {
                let seat = seat_of_in(&mut tx, game_id, user).await?;
                unseat_in(&mut tx, game_id, user).await?;
                seat_in(&mut tx, game_id, &replacement, seat, now).await?;
                drop_line_head_in(&mut tx, game_id, &replacement).await?;
                sqlx::query(
                    "INSERT OR REPLACE INTO board_line_heads (game_id, user_id, at) \
                     VALUES (?, ?, ?)",
                )
                .bind(game_id)
                .bind(user)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                bump_version_in(&mut tx, game_id).await?;
                let notice = board.notice(
                    &replacement,
                    NoticeKind::MovedIn,
                    board.board_url(),
                    (game.start, game.end),
                    None,
                );
                queue_in(&mut tx, &[notice], now).await?;
            }
        }
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Take a short game's open place by hand, whatever the fill cutoff. It joins the board if
    /// needed and adds the taker's free time over the game, so the next settle keeps them in.
    pub async fn take_place(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let seated: Vec<String> = state
            .games
            .iter()
            .find(|g| g.id == game_id)
            .map(|g| g.players.clone())
            .ok_or(StoreError::NotFound)?;
        if seated.iter().any(|p| p == user) {
            return Err(StoreError::Conflict("already_in_game"));
        }
        if seated.len() >= PLACES {
            return Err(StoreError::Conflict("game_full"));
        }
        if state.busy(user, game.start, game.end, None) {
            return Err(StoreError::Conflict("busy"));
        }
        join_in(&mut tx, &board.id, user, now).await?;
        let existing = mine(&state, user);
        let parts = uncovered_parts(&existing, game.start, game.end);
        if existing.len() + parts.len() > MAX_PIECES_PER_MEMBER {
            return Err(StoreError::Conflict("too_many_ranges"));
        }
        for (start, end) in parts {
            insert_piece_in(&mut tx, &board.id, user, start, end, now).await?;
        }
        drop_line_head_in(&mut tx, game_id, user).await?;
        let taken: Vec<i64> =
            sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ?")
                .bind(game_id)
                .fetch_all(&mut *tx)
                .await?;
        let seat = (0_i64..)
            .take(PLACES)
            .find(|s| !taken.contains(s))
            .ok_or(StoreError::Conflict("game_full"))?;
        seat_in(&mut tx, game_id, user, seat, now).await?;
        bump_version_in(&mut tx, game_id).await?;
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }
}

/// One `board_requests` row.
struct RequestRow {
    id: String,
    board_id: String,
    kind: String,
    from_user: String,
    game_id: String,
    game_version: i64,
    other_game_id: Option<String>,
    other_game_version: Option<i64>,
    to_user: Option<String>,
    new_start: Option<i64>,
    new_end: Option<i64>,
}

type RequestTuple = (
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// `SELECT <a RequestTuple> FROM board_requests <rest>`, as a `&'static str`.
macro_rules! select_request {
    ($rest:literal) => {
        concat!(
            "SELECT id, board_id, kind, from_user_id, game_id, game_version, other_game_id, \
             other_game_version, to_user_id, new_starts_at, new_ends_at FROM board_requests ",
            $rest
        )
    };
}

impl RequestRow {
    fn from_tuple(t: RequestTuple) -> RequestRow {
        RequestRow {
            id: t.0,
            board_id: t.1,
            kind: t.2,
            from_user: t.3,
            game_id: t.4,
            game_version: t.5,
            other_game_id: t.6,
            other_game_version: t.7,
            to_user: t.8,
            new_start: t.9,
            new_end: t.10,
        }
    }

    fn kind(&self) -> Result<RequestKind, StoreError> {
        match self.kind.as_str() {
            "swap" => Ok(RequestKind::Swap),
            "time" => Ok(RequestKind::Time),
            other => Err(StoreError::Corrupt(format!("request kind {other}"))),
        }
    }

    /// The proposed time of a time request.
    fn new_time(&self) -> Result<(i64, i64), StoreError> {
        self.new_start
            .zip(self.new_end)
            .ok_or_else(|| StoreError::Corrupt(format!("time request {} has no time", self.id)))
    }

    /// A request is stale once either game changed since it was asked, started, or was played.
    /// Stale requests are hidden on read, refused on answer and set `expired` by the tick.
    fn stale(&self, game: &GameRow, other: Option<&GameRow>, now: i64) -> bool {
        let moved_on = |g: &GameRow, version: Option<i64>| {
            Some(g.version) != version || g.start <= now || g.played
        };
        moved_on(game, Some(self.game_version))
            || match (&self.other_game_id, other) {
                (Some(_), Some(o)) => moved_on(o, self.other_game_version),
                (Some(_), None) => true,
                (None, _) => false,
            }
    }

    /// Who must answer: a swap's asked player, a time change's other players.
    fn addressees(&self, state: &BoardState) -> Vec<String> {
        match (self.kind.as_str(), &self.to_user) {
            ("swap", Some(to)) => vec![to.clone()],
            ("time", _) => state
                .games
                .iter()
                .find(|g| g.id == self.game_id)
                .map(|g| {
                    g.players
                        .iter()
                        .filter(|p| **p != self.from_user)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}

/// The viewer's pending requests on a board: those they asked, and those they must answer,
/// leaving out stale ones. Reads only.
pub(super) async fn requests_in(
    conn: &mut SqliteConnection,
    board_id: &str,
    viewer: Option<&str>,
    state: &BoardState,
    games: &HashMap<String, GameRow>,
    names: &HashMap<String, String>,
    now: i64,
) -> Result<Vec<RequestOut>, StoreError> {
    let Some(viewer) = viewer else {
        return Ok(Vec::new());
    };
    let rows: Vec<RequestTuple> = sqlx::query_as(select_request!(
        "WHERE board_id = ? AND status = 'pending' ORDER BY created_at, rowid"
    ))
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;
    let answered: Vec<String> = sqlx::query_scalar(
        "SELECT a.request_id FROM board_request_answers a \
         JOIN board_requests r ON r.id = a.request_id \
         WHERE r.board_id = ? AND r.status = 'pending' AND a.user_id = ?",
    )
    .bind(board_id)
    .bind(viewer)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows.into_iter().map(RequestRow::from_tuple) {
        let Some(game) = games.get(&row.game_id) else {
            continue;
        };
        let other = row.other_game_id.as_ref().and_then(|id| games.get(id));
        if row.stale(game, other, now) {
            continue;
        }
        let asked = row.from_user == viewer;
        let addressee = row.addressees(state).iter().any(|u| u == viewer);
        if !asked && !addressee {
            continue;
        }
        out.push(RequestOut {
            kind: row.kind()?,
            from: MemberRef {
                user_id: row.from_user.clone(),
                name: names.get(&row.from_user).cloned().unwrap_or_default(),
                guest: super::guests::is_guest_id(&row.from_user),
            },
            game_id: row.game_id.clone(),
            other_game_id: row.other_game_id.clone(),
            new_start: row.new_start,
            new_end: row.new_end,
            needs_my_answer: addressee && !answered.contains(&row.id),
            id: row.id,
        });
    }
    Ok(out)
}

/// Add the proposed game to a notice, and whether it was accepted.
fn with_new(mut notice: Notice, new: (i64, i64), accepted: Option<bool>) -> Notice {
    notice.payload.new_start = Some(new.0);
    notice.payload.new_end = Some(new.1);
    notice.payload.accepted = accepted;
    notice
}

/// What answering a request came to.
enum Verdict {
    /// Recorded or applied: settle and show the board.
    Done,
    /// The request no longer holds: it is set `expired`, committed, and this is the answer.
    Expired(StoreError),
}

/// Move a game to `[start, end)` by hand. Each player's free time over the old time that the new
/// time does not also cover goes, or the next settle would form a second game there with them.
/// The game is `moved`, so it is no longer tied to the players' ranges. Everyone in it but
/// `skip` is told. The caller settles.
async fn apply_time_in(
    tx: &mut Tx<'_>,
    board: &BoardRow,
    game_id: &str,
    proposer: &str,
    new: (i64, i64),
    skip: &[&str],
    now: i64,
) -> Result<(), StoreError> {
    let state = load_state_in(tx, &board.id).await?;
    let game = state
        .games
        .iter()
        .find(|g| g.id == game_id)
        .ok_or(StoreError::NotFound)?;
    let (old, players) = ((game.start, game.end), game.players.clone());
    // Old minus new: at most the part before the new time and the part after it.
    let cuts = [(old.0, old.1.min(new.0)), (old.0.max(new.1), old.1)];
    for player in &players {
        let cut = trim_ranges(&mine(&state, player), &cuts);
        apply_trim_in(tx, &board.id, player, cut).await?;
    }
    sqlx::query(
        "UPDATE board_games SET starts_at = ?, ends_at = ?, moved = 1, version = version + 1 \
         WHERE id = ?",
    )
    .bind(new.0)
    .bind(new.1)
    .bind(game_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM board_line_heads WHERE game_id = ?")
        .bind(game_id)
        .execute(&mut **tx)
        .await?;
    let name = display_name_in(tx, proposer).await?;
    let notices: Vec<Notice> = players
        .iter()
        .filter(|p| !skip.contains(&p.as_str()))
        .map(|p| {
            with_new(
                board.notice(
                    p,
                    NoticeKind::TimeChanged,
                    board.board_url(),
                    old,
                    Some(name.clone()),
                ),
                new,
                None,
            )
        })
        .collect();
    queue_in(tx, &notices, now).await?;
    Ok(())
}

async fn close_request_in(
    tx: &mut Tx<'_>,
    request_id: &str,
    status: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE board_requests SET status = ?, decided_at = ? WHERE id = ?")
        .bind(status)
        .bind(now)
        .bind(request_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A failed [`check_new_time`] as the store error it is.
fn new_time_error(code: &'static str) -> StoreError {
    match code {
        "range" => StoreError::InvalidField("range"),
        other => StoreError::Conflict(other),
    }
}

async fn has_pending_in(
    tx: &mut Tx<'_>,
    kind: &str,
    user: &str,
    game_id: &str,
    version: i64,
) -> Result<bool, StoreError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM board_requests WHERE status = 'pending' AND kind = ? \
         AND from_user_id = ? AND game_id = ? AND game_version = ?",
    )
    .bind(kind)
    .bind(user)
    .bind(game_id)
    .bind(version)
    .fetch_one(&mut **tx)
    .await?;
    Ok(n > 0)
}

impl Store {
    /// Ask a player of an overlapping game to trade places. Both must be free for the other's
    /// game. Nothing changes until they accept ([`Store::answer_request`]).
    #[allow(clippy::too_many_arguments)]
    pub async fn ask_swap(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        other_game_id: &str,
        other_user_id: &str,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let other = game_row_in(&mut tx, &board.id, other_game_id).await?;
        let state = load_state_in(&mut tx, &board.id).await?;
        if !can_swap(&state, game_id, user, other_game_id, other_user_id, now) {
            return Err(StoreError::Conflict("cannot_swap"));
        }
        if has_pending_in(&mut tx, "swap", user, game_id, version).await? {
            return Err(StoreError::Conflict("request_pending"));
        }
        sqlx::query(
            "INSERT INTO board_requests (id, board_id, kind, from_user_id, game_id, game_version, \
             other_game_id, other_game_version, to_user_id, created_at) \
             VALUES (?, ?, 'swap', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&board.id)
        .bind(user)
        .bind(game_id)
        .bind(version)
        .bind(other_game_id)
        .bind(other.version)
        .bind(other_user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let name = display_name_in(&mut tx, user).await?;
        let notice = with_new(
            board.notice(
                other_user_id,
                NoticeKind::SwapAsked,
                board.board_url(),
                (other.start, other.end),
                Some(name),
            ),
            (game.start, game.end),
            None,
        );
        queue_in(&mut tx, &[notice], now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Move a game to `[start, end)`. With `agreed` (the players settled it in the chat), or when
    /// the asker is the only player, it happens at once and the others are only told. Otherwise
    /// it is a request every other player must accept.
    #[allow(clippy::too_many_arguments)]
    pub async fn ask_time(
        &self,
        token: &str,
        game_id: &str,
        user: &str,
        version: i64,
        start: i64,
        end: i64,
        agreed: bool,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let (board, game) = open_game_in(&mut tx, token, game_id, version, now).await?;
        still_open(&game, now)?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let players = state
            .games
            .iter()
            .find(|g| g.id == game_id)
            .map(|g| g.players.clone())
            .unwrap_or_default();
        if !players.iter().any(|p| p == user) {
            return Err(StoreError::Conflict("not_in_game"));
        }
        check_new_time(&state, game_id, start, end, now).map_err(new_time_error)?;
        if agreed || players.len() == 1 {
            apply_time_in(&mut tx, &board, game_id, user, (start, end), &[user], now).await?;
            settle_in(&mut tx, &board.id, Some(user), now).await?;
        } else {
            if has_pending_in(&mut tx, "time", user, game_id, version).await? {
                return Err(StoreError::Conflict("request_pending"));
            }
            sqlx::query(
                "INSERT INTO board_requests (id, board_id, kind, from_user_id, game_id, \
                 game_version, new_starts_at, new_ends_at, created_at) \
                 VALUES (?, ?, 'time', ?, ?, ?, ?, ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&board.id)
            .bind(user)
            .bind(game_id)
            .bind(version)
            .bind(start)
            .bind(end)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let name = display_name_in(&mut tx, user).await?;
            let notices: Vec<Notice> = players
                .iter()
                .filter(|p| p.as_str() != user)
                .map(|p| {
                    with_new(
                        board.notice(
                            p,
                            NoticeKind::TimeAsked,
                            board.board_url(),
                            (game.start, game.end),
                            Some(name.clone()),
                        ),
                        (start, end),
                        None,
                    )
                })
                .collect();
            queue_in(&mut tx, &notices, now).await?;
        }
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Answer a request addressed to `user`. A request that went stale (a game changed or
    /// started) or that no longer works is set `expired` and answered with its code. A swap
    /// acts on the one answer; a time change acts once every other player has accepted, and a
    /// single decline drops it.
    pub async fn answer_request(
        &self,
        request_id: &str,
        user: &str,
        accept: bool,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let row: Option<RequestTuple> =
            sqlx::query_as(select_request!("WHERE id = ? AND status = 'pending'"))
                .bind(request_id)
                .fetch_optional(&mut *tx)
                .await?;
        let request = RequestRow::from_tuple(row.ok_or(StoreError::NotFound)?);
        let board = find_by_id_in(&mut tx, &request.board_id).await?;
        let state = load_state_in(&mut tx, &board.id).await?;
        if !request.addressees(&state).iter().any(|u| u == user) {
            return Err(StoreError::NotFound);
        }
        let verdict = decide_in(&mut tx, &board, &request, &state, user, accept, now).await?;
        if let Verdict::Expired(error) = verdict {
            tx.commit().await?;
            return Err(error);
        }
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Set `expired` on every pending request whose game, or other game, changed since it was
    /// asked or has started. Tells no one. Returns how many.
    pub async fn expire_requests(&self, now: i64) -> Result<usize, StoreError> {
        let mut tx = self.begin().await?;
        let done = sqlx::query(
            "UPDATE board_requests SET status = 'expired', decided_at = ?1 \
             WHERE status = 'pending' AND ( \
               EXISTS (SELECT 1 FROM board_games g WHERE g.id = board_requests.game_id \
                 AND (g.version <> board_requests.game_version OR g.starts_at <= ?1 \
                      OR g.match_id IS NOT NULL)) \
               OR EXISTS (SELECT 1 FROM board_games o WHERE o.id = board_requests.other_game_id \
                 AND (o.version <> board_requests.other_game_version OR o.starts_at <= ?1 \
                      OR o.match_id IS NOT NULL)))",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(usize::try_from(done.rows_affected()).unwrap_or(usize::MAX))
    }
}

/// The body of [`Store::answer_request`], once the request and the answerer are known to fit.
async fn decide_in(
    tx: &mut Tx<'_>,
    board: &BoardRow,
    request: &RequestRow,
    state: &BoardState,
    user: &str,
    accept: bool,
    now: i64,
) -> Result<Verdict, StoreError> {
    let game = game_row_in(tx, &board.id, &request.game_id).await?;
    let other = match &request.other_game_id {
        Some(id) => Some(game_row_in(tx, &board.id, id).await?),
        None => None,
    };
    let expire = |error: StoreError| Verdict::Expired(error);
    if request.stale(&game, other.as_ref(), now) {
        close_request_in(tx, &request.id, "expired", now).await?;
        return Ok(expire(StoreError::Conflict("request_expired")));
    }
    let answerer = display_name_in(tx, user).await?;
    let current = (game.start, game.end);
    match request.kind()? {
        RequestKind::Swap => {
            let (Some(other_id), Some(other)) = (&request.other_game_id, &other) else {
                return Err(StoreError::Corrupt(format!(
                    "swap {} has no other game",
                    request.id
                )));
            };
            let theirs = (other.start, other.end);
            if accept {
                if !can_swap(
                    state,
                    &request.game_id,
                    &request.from_user,
                    other_id,
                    user,
                    now,
                ) {
                    close_request_in(tx, &request.id, "expired", now).await?;
                    return Ok(expire(StoreError::Conflict("cannot_swap")));
                }
                trade_seats_in(
                    tx,
                    &request.game_id,
                    &request.from_user,
                    other_id,
                    user,
                    now,
                )
                .await?;
            }
            close_request_in(
                tx,
                &request.id,
                if accept { "accepted" } else { "declined" },
                now,
            )
            .await?;
            let notice = with_new(
                board.notice(
                    &request.from_user,
                    NoticeKind::SwapAnswered,
                    board.board_url(),
                    current,
                    Some(answerer),
                ),
                theirs,
                Some(accept),
            );
            queue_in(tx, &[notice], now).await?;
        }
        RequestKind::Time => {
            let new = request.new_time()?;
            let answers: Vec<(String, String)> = sqlx::query_as(
                "SELECT user_id, answer FROM board_request_answers WHERE request_id = ?",
            )
            .bind(&request.id)
            .fetch_all(&mut **tx)
            .await?;
            if answers.iter().any(|(u, _)| u == user) {
                return Err(StoreError::Conflict("already_answered"));
            }
            sqlx::query(
                "INSERT INTO board_request_answers (request_id, user_id, answer, at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&request.id)
            .bind(user)
            .bind(if accept { "accept" } else { "decline" })
            .bind(now)
            .execute(&mut **tx)
            .await?;
            if !accept {
                close_request_in(tx, &request.id, "declined", now).await?;
                let notice = with_new(
                    board.notice(
                        &request.from_user,
                        NoticeKind::TimeAnswered,
                        board.board_url(),
                        current,
                        Some(answerer),
                    ),
                    new,
                    Some(false),
                );
                queue_in(tx, &[notice], now).await?;
                return Ok(Verdict::Done);
            }
            let everyone = request.addressees(state).iter().all(|a| {
                a == user
                    || answers
                        .iter()
                        .any(|(u, answer)| u == a && answer == "accept")
            });
            if everyone {
                if let Err(code) = check_new_time(state, &request.game_id, new.0, new.1, now) {
                    close_request_in(tx, &request.id, "expired", now).await?;
                    return Ok(expire(new_time_error(code)));
                }
                close_request_in(tx, &request.id, "accepted", now).await?;
                let skip = [request.from_user.as_str(), user];
                apply_time_in(
                    tx,
                    board,
                    &request.game_id,
                    &request.from_user,
                    new,
                    &skip,
                    now,
                )
                .await?;
                let notice = with_new(
                    board.notice(
                        &request.from_user,
                        NoticeKind::TimeAnswered,
                        board.board_url(),
                        current,
                        None,
                    ),
                    new,
                    Some(true),
                );
                queue_in(tx, &[notice], now).await?;
            }
        }
    }
    Ok(Verdict::Done)
}
