-- 0017_board_game_holds.sql. The time a moved game left stays blocked for the players who were in
-- it when it moved: they cannot be put into another game there, though their painted free time is
-- kept. One row per player, per game and per part of the old time that the new time does not
-- cover. The cascade drops a player's holds when they leave the game or the game is deleted, and
-- follows the `user_id` rewrite when a guest claims a seat. Holds of a game a match was started
-- from (`match_id` set) are ignored, so they never block anyone for good.
-- A later migration that rebuilds `board_game_players` (DROP + RENAME, as 0014 did) with foreign
-- keys on would cascade-delete every hold: it must copy `board_game_holds` too, or rebuild with
-- foreign keys off.
CREATE TABLE board_game_holds (
    game_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    starts_at INTEGER NOT NULL,
    ends_at INTEGER NOT NULL,
    PRIMARY KEY (game_id, user_id, starts_at, ends_at),
    FOREIGN KEY (game_id, user_id) REFERENCES board_game_players(game_id, user_id)
        ON DELETE CASCADE ON UPDATE CASCADE
);
