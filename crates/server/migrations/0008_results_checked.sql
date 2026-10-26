-- 0008_results_checked.sql. The revision at which the startup results backfill last replayed a
-- match; NULL = never. A match whose revision has not moved since is not replayed again.
ALTER TABLE matches ADD COLUMN results_checked_revision INTEGER;
