ALTER TABLE players ADD COLUMN linked_user_id TEXT REFERENCES users(id);   -- NULL = a plain name
ALTER TABLE players ADD COLUMN linked_at INTEGER;
CREATE INDEX players_linked ON players(linked_user_id) WHERE linked_user_id IS NOT NULL;

ALTER TABLE users ADD COLUMN history_visibility TEXT NOT NULL DEFAULT 'coordinators'
  CHECK (history_visibility IN ('private', 'coordinators', 'public'));
ALTER TABLE users ADD COLUMN profile_token TEXT;                           -- set while public
CREATE UNIQUE INDEX users_profile_token ON users(profile_token) WHERE profile_token IS NOT NULL;

CREATE TABLE player_claims (
  id TEXT PRIMARY KEY,
  player_id TEXT NOT NULL REFERENCES players(id),
  claimant_user_id TEXT NOT NULL REFERENCES users(id),
  owner_user_id TEXT NOT NULL,                 -- the entry's coordinator, for the inbox query
  status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'declined', 'withdrawn')),
  created_at INTEGER NOT NULL,
  decided_at INTEGER,
  UNIQUE (player_id, claimant_user_id)         -- one row per person and entry, reused on re-claim
);
CREATE INDEX player_claims_owner ON player_claims(owner_user_id, status, created_at);
CREATE INDEX player_claims_claimant ON player_claims(claimant_user_id, created_at);

CREATE TABLE link_codes (
  code_hash TEXT PRIMARY KEY,                  -- SHA-256 hex, never the raw code
  player_id TEXT NOT NULL REFERENCES players(id),
  created_by TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
  used_by TEXT REFERENCES users(id), used_at INTEGER
);
CREATE INDEX link_codes_player ON link_codes(player_id);
