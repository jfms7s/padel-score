-- 0018_push_retry.sql. Web push retry back-off and refused-subscription clean-up.
-- `push_subscriptions.failed_since`: when the push service started answering 401/403 without a
-- success in between (NULL when it is not refusing); a success or a re-subscribe clears it, and a
-- refusal that comes a week after it deletes the subscription.
-- `push_outbox.next_attempt_at`: the earliest instant a failed notice is tried again (NULL: now).
-- The partial index `push_outbox_pending` still serves the pending scan; the wait is a filter on
-- the unsent rows it finds. No backfill: both columns start empty.
ALTER TABLE push_subscriptions ADD COLUMN failed_since INTEGER;
ALTER TABLE push_outbox ADD COLUMN next_attempt_at INTEGER;
