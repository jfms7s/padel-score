-- 0006_tournament.sql. One row per match of a tournament's bracket, made at Start. A draw
-- covers places place_from .. place_from + size - 1; its match idx meets slots 2*idx (team_a)
-- and 2*idx + 1 (team_b). match_id is NULL until both teams are known and the event match
-- exists.
CREATE TABLE bracket_matches (
  event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  place_from INTEGER NOT NULL,
  size INTEGER NOT NULL,
  idx INTEGER NOT NULL,
  team_a_id TEXT REFERENCES competition_teams(id),
  team_b_id TEXT REFERENCES competition_teams(id),
  match_id TEXT UNIQUE REFERENCES matches(id) ON DELETE SET NULL,
  held INTEGER NOT NULL DEFAULT 0,   -- 1: its winner changed after a match it feeds started
  PRIMARY KEY (event_id, place_from, size, idx)
);
CREATE INDEX bracket_matches_match ON bracket_matches(match_id);
