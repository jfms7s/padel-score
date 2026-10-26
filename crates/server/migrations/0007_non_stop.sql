-- 0007_non_stop.sql. A slot is one clock run: one wave of one round. Clock times are unix ms.
CREATE TABLE non_stop_slots (
  event_id TEXT NOT NULL REFERENCES competitions(id) ON DELETE CASCADE,
  slot INTEGER NOT NULL,                     -- 1-based, in play order
  round INTEGER NOT NULL,                    -- 1-based across legs
  wave INTEGER NOT NULL,                     -- 1-based within the round
  state TEXT NOT NULL DEFAULT 'ready' CHECK (state IN ('ready', 'running', 'paused', 'done')),
  duration_ms INTEGER NOT NULL,
  remaining_ms INTEGER NOT NULL,             -- ready/paused: time left; done: 0
  starts_at_ms INTEGER,                      -- running only: the tap (Start or Resume) + 5 s
  ends_at_ms INTEGER,                        -- running only: starts_at_ms + remaining_ms
  started_at_ms INTEGER,                     -- the first go; set once, after practice points are cleared
  done_at_ms INTEGER,
  PRIMARY KEY (event_id, slot)
);
CREATE INDEX non_stop_slots_due ON non_stop_slots(state, ends_at_ms);
CREATE TABLE non_stop_matches (
  match_id TEXT PRIMARY KEY REFERENCES competition_matches(match_id) ON DELETE CASCADE,
  event_id TEXT NOT NULL,
  slot INTEGER NOT NULL,
  leg INTEGER NOT NULL,
  needs_time_up INTEGER NOT NULL DEFAULT 0,  -- 1: the runner must check this match's time-up
  FOREIGN KEY (event_id, slot) REFERENCES non_stop_slots(event_id, slot) ON DELETE CASCADE
);
CREATE INDEX non_stop_matches_slot ON non_stop_matches(event_id, slot);
CREATE INDEX non_stop_matches_pending ON non_stop_matches(needs_time_up) WHERE needs_time_up = 1;
