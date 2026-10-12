-- DB2b: full-text search over song metadata.
--
-- Kept in its own database (and connection pool) so a search never contends
-- with the library writer. It duplicates the fields it needs (rather than
-- joining library.db) because cross-database joins are not available.

CREATE VIRTUAL TABLE song_search USING fts5 (
    title,
    artist,
    album,
    duration UNINDEXED,
    song_id UNINDEXED,
    tokenize = 'unicode61'
);
