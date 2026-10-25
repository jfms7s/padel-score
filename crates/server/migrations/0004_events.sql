-- 0004_events.sql. Times are unix seconds. "Event" in the UI is a competition here: the match
-- log table is already called `events`. placings.event_id (0003) holds competitions.id.
CREATE TABLE competitions (
  id TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL,                 -- no FK, as for matches.owner_user_id
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('tournament', 'non_stop')),
  format TEXT NOT NULL,                        -- JSON padel_scoring::Format
  settings TEXT NOT NULL DEFAULT '{}',         -- JSON owned by the kind module (phases 7, 8)
  status TEXT NOT NULL DEFAULT 'setup' CHECK (status IN ('setup', 'running', 'finished')),
  spectator_token TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER
);
CREATE INDEX competitions_owner ON competitions(owner_user_id, created_at DESC);
CREATE TABLE competition_teams (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  name TEXT,                                   -- NULL: shown as "Ana / Marta"
  player1_id TEXT NOT NULL REFERENCES players(id),
  player2_id TEXT NOT NULL REFERENCES players(id),
  position INTEGER NOT NULL,
  CHECK (player1_id <> player2_id)
);
CREATE INDEX competition_teams_event ON competition_teams(event_id, position);
CREATE TABLE courts (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  scorer_token TEXT NOT NULL UNIQUE,
  spectator_token TEXT NOT NULL UNIQUE,
  current_match_id TEXT UNIQUE REFERENCES matches(id) ON DELETE SET NULL
);
CREATE INDEX courts_event ON courts(event_id, position);
CREATE TABLE competition_matches (
  match_id TEXT PRIMARY KEY REFERENCES matches(id) ON DELETE CASCADE,
  event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  team_a_id TEXT NOT NULL REFERENCES competition_teams(id),
  team_b_id TEXT NOT NULL REFERENCES competition_teams(id),
  label TEXT NOT NULL,
  ordinal INTEGER NOT NULL,                    -- creation order within the event
  court_id TEXT REFERENCES courts(id),
  court_seq INTEGER,                           -- queue order on court_id
  CHECK (team_a_id <> team_b_id),
  UNIQUE (event_id, ordinal)
);
CREATE INDEX competition_matches_event ON competition_matches(event_id, ordinal);
CREATE INDEX competition_matches_court ON competition_matches(court_id, court_seq);
ALTER TABLE matches ADD COLUMN result_locked_at INTEGER;
