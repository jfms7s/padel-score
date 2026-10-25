CREATE TABLE players (
  id TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL,
  name TEXT NOT NULL,
  name_key TEXT NOT NULL,
  archived_at INTEGER,
  created_at INTEGER NOT NULL,
  UNIQUE (owner_user_id, name_key)
);

CREATE TABLE match_players (
  match_id TEXT NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
  team TEXT NOT NULL CHECK (team IN ('a', 'b')),
  position INTEGER NOT NULL CHECK (position IN (0, 1)),
  player_id TEXT NOT NULL REFERENCES players(id),
  PRIMARY KEY (match_id, team, position),
  UNIQUE (match_id, player_id)
);
CREATE INDEX match_players_player ON match_players(player_id);

CREATE TABLE match_results (
  match_id TEXT PRIMARY KEY REFERENCES matches(id) ON DELETE CASCADE,
  owner_user_id TEXT NOT NULL,
  winner TEXT CHECK (winner IN ('a', 'b')),
  score TEXT NOT NULL,
  finished_at INTEGER NOT NULL
);
CREATE INDEX match_results_owner ON match_results(owner_user_id, finished_at DESC, match_id DESC);

CREATE TABLE placings (
  player_id TEXT NOT NULL REFERENCES players(id),
  event_id TEXT NOT NULL,
  event_kind TEXT NOT NULL CHECK (event_kind IN ('tournament', 'non_stop')),
  place INTEGER NOT NULL CHECK (place >= 1),
  place_to INTEGER NOT NULL,
  field_size INTEGER NOT NULL,
  finished_at INTEGER NOT NULL,
  PRIMARY KEY (player_id, event_id),
  CHECK (place_to >= place)
);
