-- 0011_boards.sql. Game boards: a week of shared free time, the games formed from it, the
-- line for a place in each game, and requests to swap places or move a game.
CREATE TABLE boards (
  id TEXT PRIMARY KEY, token TEXT NOT NULL UNIQUE,
  creator_user_id TEXT NOT NULL REFERENCES users(id),
  title TEXT NOT NULL, place TEXT, time_zone TEXT NOT NULL,
  starts_at INTEGER NOT NULL, ends_at INTEGER NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX boards_creator ON boards(creator_user_id, ends_at);
CREATE TABLE board_members (
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id), joined_at INTEGER NOT NULL,
  PRIMARY KEY (board_id, user_id));
CREATE INDEX board_members_user ON board_members(user_id);
CREATE TABLE board_ranges (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL, user_id TEXT NOT NULL,
  starts_at INTEGER NOT NULL, ends_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
  CHECK (ends_at > starts_at),
  FOREIGN KEY (board_id, user_id) REFERENCES board_members(board_id, user_id) ON DELETE CASCADE);
CREATE INDEX board_ranges_board ON board_ranges(board_id, user_id, starts_at);
CREATE TABLE board_games (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  starts_at INTEGER NOT NULL, ends_at INTEGER NOT NULL, formed_at INTEGER NOT NULL,
  moved INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 0,
  match_id TEXT REFERENCES matches(id) ON DELETE SET NULL, played_at INTEGER);
CREATE INDEX board_games_board ON board_games(board_id, starts_at);
CREATE TABLE board_game_players (
  game_id TEXT NOT NULL REFERENCES board_games(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id), seat INTEGER NOT NULL, joined_at INTEGER NOT NULL,
  PRIMARY KEY (game_id, user_id));
CREATE INDEX board_game_players_user ON board_game_players(user_id);
CREATE TABLE board_line_heads (
  game_id TEXT NOT NULL REFERENCES board_games(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (game_id, user_id));
CREATE TABLE board_requests (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('swap', 'time')),
  from_user_id TEXT NOT NULL, game_id TEXT NOT NULL REFERENCES board_games(id) ON DELETE CASCADE,
  game_version INTEGER NOT NULL,
  other_game_id TEXT REFERENCES board_games(id) ON DELETE CASCADE, other_game_version INTEGER,
  to_user_id TEXT, new_starts_at INTEGER, new_ends_at INTEGER,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','accepted','declined','expired')),
  created_at INTEGER NOT NULL, decided_at INTEGER);
CREATE INDEX board_requests_pending ON board_requests(status, board_id);
CREATE TABLE board_request_answers (
  request_id TEXT NOT NULL REFERENCES board_requests(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL, answer TEXT NOT NULL CHECK (answer IN ('accept','decline')), at INTEGER NOT NULL,
  PRIMARY KEY (request_id, user_id));
