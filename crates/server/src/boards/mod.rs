//! Game boards: a week of shared free time. Members add ranges of free time, and games form
//! from them by the pure rules in [`formation`]. This module is the store around those rules:
//! it loads a board into a [`BoardState`], applies the [`Step`]s the rules return and queues the
//! notices, all inside the caller's `BEGIN IMMEDIATE` transaction.
//!
//! Every write ends with [`settle_in`], so no write path fills or forms a game on its own.

mod actions;
pub mod formation;
pub mod ranges;

use actions::{GameRow, leave_in, requests_in};
pub(crate) use calendar::{calendar_games_in, queue_short_games_in, waiting_and_strips_in};

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use crate::push::{Notice, NoticeKind, Payload, queue_in};
use crate::store::{MAX_NAME_CHARS, Store, StoreError, Tx, normalize_place, present};
use crate::tokens::new_token;
use formation::{BoardState, Game, Member, PLACES, SLOT_SECS, Step, settle, uncovered};
use ranges::{Piece, trim, uncovered_parts};

pub const MAX_BOARD_MEMBERS: usize = 64;
pub const MAX_PIECES_PER_MEMBER: usize = 200;
pub const MAX_OPEN_BOARDS_PER_CREATOR: usize = 20;
pub const WEEK_SECS: i64 = 604_800;
pub const WEEK_SLACK_SECS: i64 = 3600;
pub const MAX_TZ_CHARS: usize = 64;

#[derive(Debug, Deserialize)]
pub struct NewBoard {
    pub title: String,
    pub place: Option<String>,
    pub time_zone: String,
    pub starts_at: i64,
    pub ends_at: i64,
}

/// A board edit. `place` is `None` when absent, `Some(None)` for an explicit `null`.
#[derive(Debug, Deserialize, Default)]
pub struct BoardChange {
    pub title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub place: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameState {
    Full,
    Short,
    /// A match was started from it.
    Played,
    /// Ended without a match.
    Past,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardOut {
    pub id: String,
    pub token: String,
    pub title: String,
    pub place: Option<String>,
    pub time_zone: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub creator_user_id: String,
    pub creator_name: String,
    pub over: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberOut {
    pub user_id: String,
    pub name: String,
    pub joined_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PieceOut {
    pub user_id: String,
    pub start: i64,
    pub end: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberRef {
    pub user_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameOut {
    pub id: String,
    pub start: i64,
    pub end: i64,
    pub version: i64,
    pub moved: bool,
    pub state: GameState,
    /// In seat order.
    pub players: Vec<MemberRef>,
    /// User ids next in line for a place. Empty once the game is played or past.
    pub line: Vec<String>,
    pub match_spectator_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    Swap,
    Time,
}

/// A pending swap or time-change request the viewer asked or must answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestOut {
    pub id: String,
    pub kind: RequestKind,
    pub from: MemberRef,
    pub game_id: String,
    pub other_game_id: Option<String>,
    pub new_start: Option<i64>,
    pub new_end: Option<i64>,
    pub needs_my_answer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeOut {
    pub user_id: String,
    pub member: bool,
    pub creator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardView {
    pub board: BoardOut,
    pub me: Option<MeOut>,
    pub members: Vec<MemberOut>,
    pub pieces: Vec<PieceOut>,
    pub games: Vec<GameOut>,
    pub requests: Vec<RequestOut>,
}

/// One `boards` row with the creator's display name.
struct BoardRow {
    id: String,
    token: String,
    creator_user_id: String,
    creator_name: String,
    title: String,
    place: Option<String>,
    time_zone: String,
    starts_at: i64,
    ends_at: i64,
}

type BoardTuple = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    i64,
);

/// `SELECT <a BoardTuple> FROM boards b JOIN users u <rest>`, as a `&'static str`.
macro_rules! select_board {
    ($rest:literal) => {
        concat!(
            "SELECT b.id, b.token, b.creator_user_id, u.display_name, b.title, b.place, \
             b.time_zone, b.starts_at, b.ends_at \
             FROM boards b JOIN users u ON u.id = b.creator_user_id ",
            $rest
        )
    };
}

mod calendar;

impl BoardRow {
    fn from_tuple(t: BoardTuple) -> BoardRow {
        BoardRow {
            id: t.0,
            token: t.1,
            creator_user_id: t.2,
            creator_name: t.3,
            title: t.4,
            place: t.5,
            time_zone: t.6,
            starts_at: t.7,
            ends_at: t.8,
        }
    }

    fn over(&self, now: i64) -> bool {
        self.ends_at <= now
    }

    fn out(&self, now: i64) -> BoardOut {
        BoardOut {
            id: self.id.clone(),
            token: self.token.clone(),
            title: self.title.clone(),
            place: self.place.clone(),
            time_zone: self.time_zone.clone(),
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            creator_user_id: self.creator_user_id.clone(),
            creator_name: self.creator_name.clone(),
            over: self.over(now),
        }
    }

    /// A notice about the game `[start, end)` on this board.
    fn notice(
        &self,
        user: &str,
        kind: NoticeKind,
        url: String,
        game: (i64, i64),
        name: Option<String>,
    ) -> Notice {
        Notice {
            user_id: user.to_owned(),
            payload: Payload {
                kind,
                url,
                title: Some(self.title.clone()),
                start: Some(game.0),
                end: Some(game.1),
                name,
                ..Payload::default()
            },
            dedupe: None,
        }
    }

    fn board_url(&self) -> String {
        format!("/b/{}", self.token)
    }
}

async fn find_by_token_in(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<Option<BoardRow>, StoreError> {
    let row: Option<BoardTuple> = sqlx::query_as(select_board!("WHERE b.token = ?"))
        .bind(token)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map(BoardRow::from_tuple))
}

async fn find_by_id_in(conn: &mut SqliteConnection, id: &str) -> Result<BoardRow, StoreError> {
    let row: Option<BoardTuple> = sqlx::query_as(select_board!("WHERE b.id = ?"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.map(BoardRow::from_tuple).ok_or(StoreError::NotFound)
}

/// The board for a write: unknown is `NotFound`, over is `board_over`.
async fn open_board_in(
    conn: &mut SqliteConnection,
    token: &str,
    now: i64,
) -> Result<BoardRow, StoreError> {
    let board = find_by_token_in(conn, token)
        .await?
        .ok_or(StoreError::NotFound)?;
    if board.over(now) {
        return Err(StoreError::Conflict("board_over"));
    }
    Ok(board)
}

async fn is_member_in(
    conn: &mut SqliteConnection,
    board_id: &str,
    user: &str,
) -> Result<bool, StoreError> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM board_members WHERE board_id = ? AND user_id = ?")
            .bind(board_id)
            .bind(user)
            .fetch_one(&mut *conn)
            .await?;
    Ok(n > 0)
}

/// Add the user as a member if they are not one yet. A newcomer to a full board is refused.
async fn join_in(
    conn: &mut SqliteConnection,
    board_id: &str,
    user: &str,
    now: i64,
) -> Result<(), StoreError> {
    if is_member_in(conn, board_id, user).await? {
        return Ok(());
    }
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_members WHERE board_id = ?")
        .bind(board_id)
        .fetch_one(&mut *conn)
        .await?;
    if usize::try_from(members).unwrap_or(usize::MAX) >= MAX_BOARD_MEMBERS {
        return Err(StoreError::Conflict("board_full"));
    }
    sqlx::query("INSERT INTO board_members (board_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(board_id)
        .bind(user)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn normalize_title(raw: &str) -> Result<String, StoreError> {
    let title = raw.trim();
    if title.is_empty() || title.chars().count() > MAX_NAME_CHARS {
        return Err(StoreError::InvalidField("title"));
    }
    Ok(title.to_owned())
}

fn check_time_zone(zone: &str) -> Result<(), StoreError> {
    let len = zone.chars().count();
    let ok = (1..=MAX_TZ_CHARS).contains(&len)
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'));
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidField("time_zone"))
    }
}

/// `[start, end)` inside the board's week and on its half-hour grid.
fn check_range(board: &BoardRow, start: i64, end: i64) -> Result<(), StoreError> {
    let ok = start < end
        && start >= board.starts_at
        && end <= board.ends_at
        && (start - board.starts_at) % SLOT_SECS == 0
        && (end - board.starts_at) % SLOT_SECS == 0;
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidField("range"))
    }
}

/// A board and everything the rules need, as a snapshot. Players are in seat order, a game's
/// line heads by `(at, user_id)`.
pub(crate) async fn load_state_in(
    conn: &mut SqliteConnection,
    board_id: &str,
) -> Result<BoardState, StoreError> {
    let (starts_at, ends_at): (i64, i64) =
        sqlx::query_as("SELECT starts_at, ends_at FROM boards WHERE id = ?")
            .bind(board_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(StoreError::NotFound)?;
    let members: Vec<(String, i64)> = sqlx::query_as(
        "SELECT user_id, joined_at FROM board_members WHERE board_id = ? \
         ORDER BY joined_at, user_id",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;
    let pieces: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, user_id, starts_at, ends_at, created_at FROM board_ranges \
         WHERE board_id = ? ORDER BY user_id, starts_at, id",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;
    let games: Vec<(String, i64, i64, i64, i64, bool)> = sqlx::query_as(
        "SELECT id, starts_at, ends_at, formed_at, moved, match_id IS NOT NULL FROM board_games \
         WHERE board_id = ? ORDER BY starts_at, formed_at, rowid",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;
    let seats: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.game_id, p.user_id FROM board_game_players p \
         JOIN board_games g ON g.id = p.game_id WHERE g.board_id = ? \
         ORDER BY p.game_id, p.seat",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;
    let heads: Vec<(String, String)> = sqlx::query_as(
        "SELECT h.game_id, h.user_id FROM board_line_heads h \
         JOIN board_games g ON g.id = h.game_id WHERE g.board_id = ? \
         ORDER BY h.game_id, h.at, h.user_id",
    )
    .bind(board_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut players: HashMap<String, Vec<String>> = HashMap::new();
    for (game, user) in seats {
        players.entry(game).or_default().push(user);
    }
    let mut line_heads: HashMap<String, Vec<String>> = HashMap::new();
    for (game, user) in heads {
        line_heads.entry(game).or_default().push(user);
    }
    Ok(BoardState {
        starts_at,
        ends_at,
        members: members
            .into_iter()
            .map(|(user_id, joined_at)| Member { user_id, joined_at })
            .collect(),
        pieces: pieces
            .into_iter()
            .map(|(id, user_id, start, end, created_at)| Piece {
                id,
                user_id,
                start,
                end,
                created_at,
            })
            .collect(),
        games: games
            .into_iter()
            .map(|(id, start, end, formed_at, moved, played)| Game {
                players: players.remove(&id).unwrap_or_default(),
                line_heads: line_heads.remove(&id).unwrap_or_default(),
                id,
                start,
                end,
                moved: moved != 0,
                played,
                formed_at,
            })
            .collect(),
    })
}

/// The games one write forms get stored (so listed, since rows tie on `formed_at`) by start, then
/// by their sorted player ids, not by their random ids. Fills keep their place.
fn order_forms(mut steps: Vec<Step>) -> Vec<Step> {
    let key = |step: &Step| match step {
        Step::Form { start, players, .. } => {
            let mut who = players.clone();
            who.sort();
            Some((*start, who))
        }
        Step::Fill { .. } => None,
    };
    let slots: Vec<usize> = (0..steps.len())
        .filter(|&i| steps.get(i).and_then(key).is_some())
        .collect();
    let mut forms: Vec<Step> = slots
        .iter()
        .filter_map(|&i| steps.get(i).cloned())
        .collect();
    forms.sort_by_key(|f| key(f));
    for (slot, form) in slots.into_iter().zip(forms) {
        if let Some(place) = steps.get_mut(slot) {
            *place = form;
        }
    }
    steps
}

async fn display_name_in(conn: &mut SqliteConnection, user: &str) -> Result<String, StoreError> {
    let name: Option<String> = sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?")
        .bind(user)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(name.unwrap_or_default())
}

/// Bring the board to what the rules say: players whose free time no longer covers their game
/// leave it, then short games fill from the line and new games form. Notices are queued here.
/// `actor` is the user whose action caused this: they are not told they were moved in.
pub(crate) async fn settle_in(
    tx: &mut Tx<'_>,
    board_id: &str,
    actor: Option<&str>,
    now: i64,
) -> Result<(), StoreError> {
    let state = load_state_in(tx, board_id).await?;
    let gone = uncovered(&state, now);
    for (game_id, user) in &gone {
        leave_in(tx, board_id, game_id, user, now).await?;
    }
    let state = if gone.is_empty() {
        state
    } else {
        load_state_in(tx, board_id).await?
    };
    let board = find_by_id_in(tx, board_id).await?;
    let mut notices = Vec::new();
    for step in order_forms(settle(&state, now)) {
        match step {
            Step::Form {
                start,
                end,
                players,
            } => {
                let game_id = uuid::Uuid::new_v4().to_string();
                sqlx::query(
                    "INSERT INTO board_games (id, board_id, starts_at, ends_at, formed_at) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&game_id)
                .bind(board_id)
                .bind(start)
                .bind(end)
                .bind(now)
                .execute(&mut **tx)
                .await?;
                for (seat, user) in (0_i64..).zip(&players) {
                    sqlx::query(
                        "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) \
                         VALUES (?, ?, ?, ?)",
                    )
                    .bind(&game_id)
                    .bind(user)
                    .bind(seat)
                    .bind(now)
                    .execute(&mut **tx)
                    .await?;
                    notices.push(board.notice(
                        user,
                        NoticeKind::GameFormed,
                        board.board_url(),
                        (start, end),
                        None,
                    ));
                }
            }
            Step::Fill { game_id, user_id } => {
                let taken: Vec<i64> =
                    sqlx::query_scalar("SELECT seat FROM board_game_players WHERE game_id = ?")
                        .bind(&game_id)
                        .fetch_all(&mut **tx)
                        .await?;
                let seat = (0_i64..)
                    .take(PLACES)
                    .find(|s| !taken.contains(s))
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!("game {game_id} has no free seat"))
                    })?;
                sqlx::query(
                    "INSERT INTO board_game_players (game_id, user_id, seat, joined_at) \
                     VALUES (?, ?, ?, ?)",
                )
                .bind(&game_id)
                .bind(&user_id)
                .bind(seat)
                .bind(now)
                .execute(&mut **tx)
                .await?;
                sqlx::query("UPDATE board_games SET version = version + 1 WHERE id = ?")
                    .bind(&game_id)
                    .execute(&mut **tx)
                    .await?;
                let game = state
                    .games
                    .iter()
                    .find(|g| g.id == game_id)
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!("filled game {game_id} is unknown"))
                    })?;
                if actor != Some(user_id.as_str()) {
                    notices.push(board.notice(
                        &user_id,
                        NoticeKind::MovedIn,
                        board.board_url(),
                        (game.start, game.end),
                        None,
                    ));
                }
            }
        }
    }
    queue_in(tx, &notices, now).await?;
    Ok(())
}

/// The board as `viewer` sees it. Reads only.
async fn view_in(
    conn: &mut SqliteConnection,
    board: &BoardRow,
    viewer: Option<&str>,
    now: i64,
) -> Result<BoardView, StoreError> {
    let state = load_state_in(conn, &board.id).await?;
    let names: HashMap<String, String> = sqlx::query_as(
        "SELECT id, display_name FROM users WHERE id IN ( \
           SELECT user_id FROM board_members WHERE board_id = ?1 \
           UNION SELECT p.user_id FROM board_game_players p \
             JOIN board_games g ON g.id = p.game_id WHERE g.board_id = ?1)",
    )
    .bind(&board.id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let name_of = |user: &str| names.get(user).cloned().unwrap_or_default();

    let mut members: Vec<&Member> = state.members.iter().collect();
    members.sort_by(|a, b| (a.joined_at, &a.user_id).cmp(&(b.joined_at, &b.user_id)));
    let mut pieces: Vec<&Piece> = state.pieces.iter().collect();
    pieces.sort_by(|a, b| (&a.user_id, a.start, &a.id).cmp(&(&b.user_id, b.start, &b.id)));

    let extras: Vec<(String, i64, bool, Option<String>)> = sqlx::query_as(
        "SELECT g.id, g.version, g.match_id IS NOT NULL, m.spectator_token FROM board_games g \
         LEFT JOIN matches m ON m.id = g.match_id WHERE g.board_id = ?",
    )
    .bind(&board.id)
    .fetch_all(&mut *conn)
    .await?;
    let extras: HashMap<String, (i64, bool, Option<String>)> = extras
        .into_iter()
        .map(|(id, version, played, token)| (id, (version, played, token)))
        .collect();

    let game_rows: HashMap<String, GameRow> = state
        .games
        .iter()
        .map(|g| {
            let (version, played, _) = extras.get(&g.id).cloned().unwrap_or((0, false, None));
            (
                g.id.clone(),
                GameRow {
                    start: g.start,
                    end: g.end,
                    version,
                    played,
                },
            )
        })
        .collect();
    let requests = requests_in(conn, &board.id, viewer, &state, &game_rows, &names, now).await?;

    let games = state
        .games
        .iter()
        .map(|g| {
            let (version, played, match_spectator_token) =
                extras.get(&g.id).cloned().unwrap_or((0, false, None));
            let past = g.end <= now;
            let state_of = game_state(played, g.end, g.players.len(), now);
            GameOut {
                id: g.id.clone(),
                start: g.start,
                end: g.end,
                version,
                moved: g.moved,
                state: state_of,
                players: g
                    .players
                    .iter()
                    .map(|u| MemberRef {
                        user_id: u.clone(),
                        name: name_of(u),
                    })
                    .collect(),
                line: if played || past {
                    Vec::new()
                } else {
                    state.line(&g.id)
                },
                match_spectator_token,
            }
        })
        .collect();

    Ok(BoardView {
        board: board.out(now),
        me: viewer.map(|user| MeOut {
            user_id: user.to_owned(),
            member: state.members.iter().any(|m| m.user_id == user),
            creator: board.creator_user_id == user,
        }),
        members: members
            .into_iter()
            .map(|m| MemberOut {
                user_id: m.user_id.clone(),
                name: name_of(&m.user_id),
                joined_at: m.joined_at,
            })
            .collect(),
        pieces: pieces
            .into_iter()
            .map(|p| PieceOut {
                user_id: p.user_id.clone(),
                start: p.start,
                end: p.end,
                created_at: p.created_at,
            })
            .collect(),
        games,
        requests,
    })
}

/// A game's state as of `now`: played once a match was started from it, past once it ended, else
/// full or short by its players.
pub(crate) fn game_state(played: bool, end: i64, players: usize, now: i64) -> GameState {
    if played {
        GameState::Played
    } else if end <= now {
        GameState::Past
    } else if players >= PLACES {
        GameState::Full
    } else {
        GameState::Short
    }
}

/// The user's own pieces on a board.
fn mine(state: &BoardState, user: &str) -> Vec<Piece> {
    state
        .pieces
        .iter()
        .filter(|p| p.user_id == user)
        .cloned()
        .collect()
}

impl Store {
    pub async fn create_board(
        &self,
        creator: &str,
        new: NewBoard,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let title = normalize_title(&new.title)?;
        let place = normalize_place(new.place.as_deref())?;
        check_time_zone(&new.time_zone)?;
        let week_ok = new
            .ends_at
            .saturating_sub(new.starts_at)
            .abs_diff(WEEK_SECS)
            <= WEEK_SLACK_SECS.unsigned_abs()
            && new.ends_at > now;
        if !week_ok {
            return Err(StoreError::InvalidField("week"));
        }
        let mut tx = self.begin().await?;
        let open: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM boards WHERE creator_user_id = ? AND ends_at > ?",
        )
        .bind(creator)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        if usize::try_from(open).unwrap_or(usize::MAX) >= MAX_OPEN_BOARDS_PER_CREATOR {
            return Err(StoreError::Conflict("too_many_boards"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let token = new_token();
        sqlx::query(
            "INSERT INTO boards (id, token, creator_user_id, title, place, time_zone, starts_at, \
             ends_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&token)
        .bind(creator)
        .bind(&title)
        .bind(&place)
        .bind(&new.time_zone)
        .bind(new.starts_at)
        .bind(new.ends_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        join_in(&mut tx, &id, creator, now).await?;
        let board = find_by_id_in(&mut tx, &id).await?;
        let view = view_in(&mut tx, &board, Some(creator), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// The board by its link token, or `None`. Reads only: nothing is formed or filled here.
    pub async fn board_view(
        &self,
        token: &str,
        viewer: Option<&str>,
        now: i64,
    ) -> Result<Option<BoardView>, StoreError> {
        // One read transaction, so every query sees the same snapshot.
        let mut tx = self.pool().begin().await?;
        let Some(board) = find_by_token_in(&mut tx, token).await? else {
            return Ok(None);
        };
        Ok(Some(view_in(&mut tx, &board, viewer, now).await?))
    }

    /// Join the board; joining again is a no-op, even when the board is full.
    pub async fn join_board(
        &self,
        token: &str,
        user: &str,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        join_in(&mut tx, &board.id, user, now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Leave the board: free time goes, and so does the place in every game that has no match and has not ended,
    /// started or not (the line fills it). Played and ended games stay as history.
    pub async fn leave_board(&self, token: &str, user: &str, now: i64) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if !is_member_in(&mut tx, &board.id, user).await? {
            return Ok(());
        }
        sqlx::query("DELETE FROM board_ranges WHERE board_id = ? AND user_id = ?")
            .bind(&board.id)
            .bind(user)
            .execute(&mut *tx)
            .await?;
        let state = load_state_in(&mut tx, &board.id).await?;
        for game in state
            .games
            .iter()
            .filter(|g| !g.played && g.end > now && g.players.iter().any(|p| p == user))
        {
            leave_in(&mut tx, &board.id, &game.id, user, now).await?;
        }
        sqlx::query("DELETE FROM board_members WHERE board_id = ? AND user_id = ?")
            .bind(&board.id)
            .bind(user)
            .execute(&mut *tx)
            .await?;
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Add free time `[start, end)`; joins the board if needed. Only the parts the user does not
    /// already cover become new pieces, created now.
    pub async fn add_free(
        &self,
        token: &str,
        user: &str,
        start: i64,
        end: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        check_range(&board, start, end)?;
        join_in(&mut tx, &board.id, user, now).await?;
        let state = load_state_in(&mut tx, &board.id).await?;
        let existing = mine(&state, user);
        let parts = uncovered_parts(&existing, start, end);
        if existing.len() + parts.len() > MAX_PIECES_PER_MEMBER {
            return Err(StoreError::Conflict("too_many_ranges"));
        }
        for (part_start, part_end) in parts {
            insert_piece_in(&mut tx, &board.id, user, part_start, part_end, now).await?;
        }
        settle_in(&mut tx, &board.id, Some(user), now).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Remove free time `[start, end)`. A non-member has none to remove, so nothing changes.
    pub async fn remove_free(
        &self,
        token: &str,
        user: &str,
        start: i64,
        end: i64,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        check_range(&board, start, end)?;
        if is_member_in(&mut tx, &board.id, user).await? {
            let state = load_state_in(&mut tx, &board.id).await?;
            let existing = mine(&state, user);
            let cut = trim(&existing, start, end);
            if (existing.len().saturating_sub(cut.delete.len())) + cut.insert.len()
                > MAX_PIECES_PER_MEMBER
            {
                return Err(StoreError::Conflict("too_many_ranges"));
            }
            for id in &cut.delete {
                sqlx::query("DELETE FROM board_ranges WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            for (piece_start, piece_end, created_at) in cut.insert {
                insert_piece_in(&mut tx, &board.id, user, piece_start, piece_end, created_at)
                    .await?;
            }
            settle_in(&mut tx, &board.id, Some(user), now).await?;
        }
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Rename the board or change its place. Creator only.
    pub async fn update_board(
        &self,
        token: &str,
        user: &str,
        change: BoardChange,
        now: i64,
    ) -> Result<BoardView, StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if board.creator_user_id != user {
            return Err(StoreError::Conflict("creator_only"));
        }
        let title = match change.title {
            Some(raw) => normalize_title(&raw)?,
            None => board.title.clone(),
        };
        let place = match change.place {
            Some(raw) => normalize_place(raw.as_deref())?,
            None => board.place.clone(),
        };
        sqlx::query("UPDATE boards SET title = ?, place = ? WHERE id = ?")
            .bind(&title)
            .bind(&place)
            .bind(&board.id)
            .execute(&mut *tx)
            .await?;
        let board = find_by_id_in(&mut tx, &board.id).await?;
        let view = view_in(&mut tx, &board, Some(user), now).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Delete the board with everything on it. Creator only. Players of games that have not ended
    /// are told, except the creator.
    pub async fn delete_board(&self, token: &str, user: &str, now: i64) -> Result<(), StoreError> {
        let mut tx = self.begin().await?;
        let board = open_board_in(&mut tx, token, now).await?;
        if board.creator_user_id != user {
            return Err(StoreError::Conflict("creator_only"));
        }
        let state = load_state_in(&mut tx, &board.id).await?;
        let notices: Vec<Notice> = state
            .games
            .iter()
            .filter(|g| g.end > now)
            .flat_map(|g| {
                g.players
                    .iter()
                    .filter(|p| p.as_str() != user)
                    .map(|p| (p, g.start, g.end))
            })
            .map(|(p, start, end)| {
                board.notice(
                    p,
                    NoticeKind::BoardDeleted,
                    "/calendar".to_owned(),
                    (start, end),
                    None,
                )
            })
            .collect();
        queue_in(&mut tx, &notices, now).await?;
        sqlx::query("DELETE FROM boards WHERE id = ?")
            .bind(&board.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Boards not yet over that the user is a member of, soonest first.
    pub async fn my_boards(&self, user: &str, now: i64) -> Result<Vec<BoardOut>, StoreError> {
        let rows: Vec<BoardTuple> = sqlx::query_as(select_board!(
            "WHERE b.ends_at > ? AND b.id IN \
             (SELECT board_id FROM board_members WHERE user_id = ?) \
             ORDER BY b.starts_at, b.id"
        ))
        .bind(now)
        .bind(user)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|t| BoardRow::from_tuple(t).out(now))
            .collect())
    }
}

async fn insert_piece_in(
    tx: &mut Tx<'_>,
    board_id: &str,
    user: &str,
    start: i64,
    end: i64,
    created_at: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO board_ranges (id, board_id, user_id, starts_at, ends_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(board_id)
    .bind(user)
    .bind(start)
    .bind(end)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod order_tests {
    use super::*;

    fn form(start: i64, who: [&str; 4]) -> Step {
        Step::Form {
            start,
            end: start + 3600,
            players: who.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn simultaneous_games_are_ordered_by_start_then_sorted_player_ids() {
        let late = form(100, ["z", "y", "x", "w"]);
        let early_b = form(50, ["m", "n", "o", "p"]);
        let early_a = form(50, ["d", "c", "b", "a"]);
        let fill = Step::Fill {
            game_id: "g".into(),
            user_id: "u".into(),
        };
        let a = order_forms(vec![
            late.clone(),
            fill.clone(),
            early_b.clone(),
            early_a.clone(),
        ]);
        let b = order_forms(vec![
            early_a.clone(),
            early_b.clone(),
            late.clone(),
            fill.clone(),
        ]);
        // Forms are sorted whatever order they came in; a fill keeps its slot.
        assert_eq!(
            a,
            vec![early_a.clone(), fill.clone(), early_b.clone(), late.clone()]
        );
        assert_eq!(b, vec![early_a, early_b, late, fill]);
    }
}
