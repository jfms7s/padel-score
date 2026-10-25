-- Accounts. Times are unix seconds. Every secret column holds a SHA-256 hex, never the raw value.
-- Users are disabled, never deleted. matches.owner_user_id (from 0001) gets no FK: SQLite cannot
-- add one without rebuilding the table, so ownership is enforced in the app.
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,  -- trimmed and lowercased by the app (NOCASE is ASCII-only)
  display_name TEXT NOT NULL, password_hash TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0,
  disabled_at INTEGER,
  invited_by TEXT REFERENCES users(id),
  created_at INTEGER NOT NULL
);
CREATE TABLE sessions (
  token_hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE TABLE invites (
  id TEXT PRIMARY KEY, code_hash TEXT NOT NULL UNIQUE,
  created_by TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
  used_by TEXT REFERENCES users(id), used_at INTEGER,
  revoked_at INTEGER
);
CREATE TABLE device_codes (
  id TEXT PRIMARY KEY,
  user_code TEXT NOT NULL UNIQUE,  -- shown on the watch and typed by a person; not a secret alone
  device_code_hash TEXT NOT NULL UNIQUE,
  device_name TEXT NOT NULL,
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, last_polled_at INTEGER,
  approved_user_id TEXT REFERENCES users(id) ON DELETE CASCADE, approved_at INTEGER,
  denied_at INTEGER, consumed_at INTEGER
);
CREATE TABLE device_tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
  prefix TEXT NOT NULL,  -- the first 8 chars of the raw token (`pdl_` + 4), for display
  created_at INTEGER NOT NULL, last_used_at INTEGER, revoked_at INTEGER
);
CREATE INDEX device_tokens_user ON device_tokens(user_id);
CREATE TABLE password_resets (
  code_hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_by TEXT NOT NULL REFERENCES users(id),
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, used_at INTEGER
);
CREATE INDEX matches_owner_activity ON matches(owner_user_id, last_activity_at);
