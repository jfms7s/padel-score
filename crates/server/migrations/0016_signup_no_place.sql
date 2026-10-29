-- 0016_signup_no_place.sql. When an event sign-up was told it has no place (`no_place`): kept on
-- the row because the push outbox is pruned, and a later take must still tell the entry it is in.
ALTER TABLE event_signups ADD COLUMN no_place_at INTEGER;
