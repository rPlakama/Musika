-- S-9: song_search carries the server play count so the empty-query surface
-- ("Most played") and the typed-hit usage boost can order on it without a
-- cross-database join. The table is a disposable cache, so it is recreated
-- here; the next sync's rebuild repopulates it from library.db.

DROP TABLE IF EXISTS song_search;

CREATE VIRTUAL TABLE song_search USING fts5 (
    title,
    artist,
    album,
    duration UNINDEXED,
    song_id UNINDEXED,
    play_count UNINDEXED,
    tokenize = 'unicode61'
);
