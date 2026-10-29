-- 0014_board_guests.sql. Guests on game boards: a board participant is a user id or a guest id
-- (`guest_<uuid>`), so the board tables stop referencing users(id).
ALTER TABLE boards ADD COLUMN allow_guests INTEGER NOT NULL DEFAULT 0;

CREATE TABLE board_guests (
  id TEXT PRIMARY KEY CHECK (substr(id, 1, 6) = 'guest_'),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL, name_key TEXT NOT NULL,
  created_at INTEGER NOT NULL, gone_at INTEGER);
CREATE UNIQUE INDEX board_guests_live_name ON board_guests(board_id, name_key) WHERE gone_at IS NULL;
CREATE TABLE board_guest_keys (
  key_hash TEXT PRIMARY KEY,
  guest_id TEXT NOT NULL REFERENCES board_guests(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL);
CREATE INDEX board_guest_keys_guest ON board_guest_keys(guest_id);

CREATE TABLE board_members_new (
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL, joined_at INTEGER NOT NULL,
  PRIMARY KEY (board_id, user_id));
CREATE TABLE board_ranges_new (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL, user_id TEXT NOT NULL,
  starts_at INTEGER NOT NULL, ends_at INTEGER NOT NULL, created_at INTEGER NOT NULL,
  CHECK (ends_at > starts_at),
  FOREIGN KEY (board_id, user_id) REFERENCES board_members_new(board_id, user_id) ON DELETE CASCADE);
CREATE TABLE board_game_players_new (
  game_id TEXT NOT NULL REFERENCES board_games(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL, seat INTEGER NOT NULL, joined_at INTEGER NOT NULL,
  PRIMARY KEY (game_id, user_id));
INSERT INTO board_members_new SELECT board_id, user_id, joined_at FROM board_members;
INSERT INTO board_ranges_new SELECT id, board_id, user_id, starts_at, ends_at, created_at FROM board_ranges;
INSERT INTO board_game_players_new SELECT game_id, user_id, seat, joined_at FROM board_game_players;
DROP TABLE board_ranges;
DROP TABLE board_game_players;
DROP TABLE board_members;
ALTER TABLE board_members_new RENAME TO board_members;
ALTER TABLE board_ranges_new RENAME TO board_ranges;
ALTER TABLE board_game_players_new RENAME TO board_game_players;
CREATE INDEX board_members_user ON board_members(user_id);
CREATE INDEX board_ranges_board ON board_ranges(board_id, user_id, starts_at);
CREATE INDEX board_game_players_user ON board_game_players(user_id);

-- Display names for every board participant; gone guests keep theirs for history.
CREATE VIEW board_people AS
  SELECT id, display_name AS name FROM users
  UNION ALL SELECT id, name FROM board_guests;
