-- 0013_scheduled_matches.sql. A match planned for later ("Play later"): a date, a place, four
-- places people can take through a join link, and a line for when it is full. It becomes a live
-- match when the owner starts it (`match_id`, `started_at`). An open place has no row.
CREATE TABLE scheduled_matches (
  id TEXT PRIMARY KEY, owner_user_id TEXT NOT NULL REFERENCES users(id),
  join_token TEXT NOT NULL UNIQUE, starts_at INTEGER NOT NULL, place TEXT,
  listing TEXT NOT NULL CHECK (listing IN ('private', 'public')),
  format TEXT NOT NULL, created_at INTEGER NOT NULL, cancelled_at INTEGER,
  match_id TEXT REFERENCES matches(id) ON DELETE SET NULL, started_at INTEGER);
CREATE INDEX scheduled_matches_owner ON scheduled_matches(owner_user_id, starts_at);
CREATE INDEX scheduled_matches_public ON scheduled_matches(listing, starts_at);

-- A filled place has `user_id` (an account that joined, or a pre-filled entry linked to one),
-- `player_id` (a pre-filled entry of the owner's), or both.
CREATE TABLE scheduled_places (
  scheduled_id TEXT NOT NULL REFERENCES scheduled_matches(id) ON DELETE CASCADE,
  slot INTEGER NOT NULL CHECK (slot BETWEEN 0 AND 3),
  user_id TEXT REFERENCES users(id), player_id TEXT REFERENCES players(id),
  joined_at INTEGER NOT NULL, PRIMARY KEY (scheduled_id, slot));
CREATE INDEX scheduled_places_user ON scheduled_places(user_id);

CREATE TABLE scheduled_line (
  scheduled_id TEXT NOT NULL REFERENCES scheduled_matches(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id), joined_at INTEGER NOT NULL,
  PRIMARY KEY (scheduled_id, user_id));
