-- DB5: lyric cache (text only; no blob directory).
--
-- Re-fetchable content in its own pool, exactly like covers.db: the library
-- writer never contends with lyric reads, and purging is an independent domain.
-- `source = 'none'` rows are misses, kept for a TTL so the UI does not hammer
-- the server for songs that have no lyrics.

CREATE TABLE lyrics (
    song_id TEXT PRIMARY KEY,
    source TEXT NOT NULL,               -- lrc | subsonic | legacy | none
    synced INTEGER NOT NULL DEFAULT 0,  -- 1 when lines carry start_ms
    lang TEXT,
    offset_ms INTEGER NOT NULL DEFAULT 0,
    text TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL
);

CREATE TABLE lyric_line (
    song_id TEXT NOT NULL REFERENCES lyrics(song_id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    start_ms INTEGER,
    value TEXT NOT NULL,
    PRIMARY KEY (song_id, idx)
);

CREATE TABLE lyrics_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
