-- 0009_push.sql. Web push: the server's VAPID key (one row, made on first use), each signed-in
-- device's push subscription, and the outbox of notices waiting to be sent.
CREATE TABLE push_keys (id INTEGER PRIMARY KEY CHECK (id = 1), private_key TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE push_subscriptions (
  endpoint TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  p256dh TEXT NOT NULL, auth TEXT NOT NULL, created_at INTEGER NOT NULL, last_ok_at INTEGER);
CREATE INDEX push_subscriptions_user ON push_subscriptions(user_id, created_at);
CREATE TABLE push_outbox (
  id INTEGER PRIMARY KEY, user_id TEXT NOT NULL, payload TEXT NOT NULL, dedupe TEXT UNIQUE,
  attempts INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, sent_at INTEGER);
CREATE INDEX push_outbox_pending ON push_outbox(created_at) WHERE sent_at IS NULL;
