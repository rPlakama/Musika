-- Return to the FTS5 engine: recreate the table the Tantivy detour dropped.
CREATE VIRTUAL TABLE IF NOT EXISTS song_search USING fts5 (
    title,
    artist,
    album,
    duration UNINDEXED,
    song_id UNINDEXED,
    play_count UNINDEXED,
    tokenize = 'unicode61'
);
