-- 0012_event_signups.sql. Scheduled events: a date, a place, sign-up settings, a join link, a
-- cancel mark, and the sign-ups themselves. A cancelled event keeps its `status` ('setup') and
-- is read as cancelled when `cancelled_at` is set, because SQLite cannot change the status
-- CHECK without rebuilding a table that four others reference.
ALTER TABLE competitions ADD COLUMN starts_at INTEGER;
ALTER TABLE competitions ADD COLUMN place TEXT;
ALTER TABLE competitions ADD COLUMN signups TEXT NOT NULL DEFAULT 'off' CHECK (signups IN ('off', 'private', 'public'));
ALTER TABLE competitions ADD COLUMN min_teams INTEGER NOT NULL DEFAULT 4;
ALTER TABLE competitions ADD COLUMN max_teams INTEGER;
ALTER TABLE competitions ADD COLUMN deadline_at INTEGER;
ALTER TABLE competitions ADD COLUMN join_token TEXT;
ALTER TABLE competitions ADD COLUMN cancelled_at INTEGER;       -- set: reads as status 'cancelled'
ALTER TABLE competitions ADD COLUMN signups_closed_at INTEGER;  -- the deadline ran or teams were taken
ALTER TABLE competitions ADD COLUMN reminded_at INTEGER;        -- the 24h push went out
CREATE UNIQUE INDEX competitions_join_token ON competitions(join_token) WHERE join_token IS NOT NULL;
CREATE INDEX competitions_starts ON competitions(signups, starts_at);

CREATE TABLE event_signups (
  id TEXT PRIMARY KEY, event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  user1_id TEXT NOT NULL REFERENCES users(id), user1_at INTEGER NOT NULL,
  user2_id TEXT REFERENCES users(id), user2_at INTEGER,
  partner_name TEXT, partner_token TEXT UNIQUE,
  auto_paired INTEGER NOT NULL DEFAULT 0, held INTEGER NOT NULL DEFAULT 0,
  complete_at INTEGER, taken_team_id TEXT REFERENCES competition_teams(id) ON DELETE SET NULL,
  CHECK (user2_id IS NULL OR partner_name IS NULL));
CREATE INDEX event_signups_event ON event_signups(event_id, complete_at);
CREATE INDEX event_signups_user1 ON event_signups(user1_id);
CREATE INDEX event_signups_user2 ON event_signups(user2_id);
