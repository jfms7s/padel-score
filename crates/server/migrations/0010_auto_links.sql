-- 1: linked by a game, sign-up or scheduled match; privacy ignores it until the link is approved.
ALTER TABLE players ADD COLUMN link_auto INTEGER NOT NULL DEFAULT 0;
