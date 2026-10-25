CREATE TABLE matches (
  id TEXT PRIMARY KEY, scorer_token TEXT NOT NULL UNIQUE, spectator_token TEXT NOT NULL UNIQUE,
  format TEXT NOT NULL, start TEXT NOT NULL, teams TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0,
  owner_user_id TEXT,
  created_at INTEGER NOT NULL, last_activity_at INTEGER NOT NULL
);
CREATE TABLE events (
  match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL, event TEXT NOT NULL,
  client_id TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'effective',
  created_at INTEGER NOT NULL,
  PRIMARY KEY (match_id, seq)
);
CREATE INDEX events_client ON events(match_id, client_id);
CREATE TABLE command_results (
  match_id TEXT NOT NULL,
  client_id TEXT NOT NULL, status INTEGER NOT NULL, body TEXT NOT NULL,
  PRIMARY KEY (match_id, client_id)
);
