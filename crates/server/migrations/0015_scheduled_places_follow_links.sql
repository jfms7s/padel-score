-- 0015_scheduled_places_follow_links.sql. A place holding an entry of the owner's (`player_id`)
-- follows that entry's current link (`players.linked_user_id`); it never stores its own copy of
-- the account. Creation used to copy the link into `user_id`, so an unlink left the old account
-- in the match. Joining writes `user_id` with no `player_id`, and nothing sets both on purpose,
-- so every `user_id` next to a `player_id` is such a copy.
UPDATE scheduled_places SET user_id = NULL WHERE player_id IS NOT NULL;

-- A scheduled match is started while `match_id` is set; deleting the match
-- (`ON DELETE SET NULL`) makes it startable again, so `started_at` is only the time it was
-- started and is no longer read.
