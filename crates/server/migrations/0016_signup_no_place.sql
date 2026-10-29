-- 0016_signup_no_place.sql. When an event sign-up was told it has no place (`no_place`): kept on
-- the row because the push outbox is pruned, and a later take must still tell the entry it is in.
ALTER TABLE event_signups ADD COLUMN no_place_at INTEGER;
-- When the organiser first took sign-ups as teams. From then on only taken entries have a place,
-- even once every taken team is removed; `signups_closed_at` is not that, since the deadline job
-- also sets it without a take.
ALTER TABLE competitions ADD COLUMN teams_taken_at INTEGER;
