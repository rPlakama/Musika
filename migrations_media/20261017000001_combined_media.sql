-- ARC-memory: squash the media-side databases into media.db.
-- covers, lyrics and the download ledger now live beside media/media_rule so
-- one pool owns every blob cache and its bookkeeping.

CREATE TABLE IF NOT EXISTS covers (
    id            TEXT PRIMARY KEY,
    file          TEXT NOT NULL,
    hash          TEXT,
    content_type  TEXT,
    size          INTEGER,
    etag          TEXT,
    last_modified TEXT,
    fetched_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS lyrics (
    song_id    TEXT PRIMARY KEY,
    source     TEXT NOT NULL,
    synced     INTEGER NOT NULL DEFAULT 0,
    lang       TEXT,
    offset_ms  INTEGER NOT NULL DEFAULT 0,
    text       TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS lyric_line (
    song_id  TEXT NOT NULL REFERENCES lyrics(song_id) ON DELETE CASCADE,
    idx      INTEGER NOT NULL,
    start_ms INTEGER,
    value    TEXT NOT NULL,
    PRIMARY KEY (song_id, idx)
);

CREATE TABLE IF NOT EXISTS lyrics_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS download (
    song_id      TEXT PRIMARY KEY,
    state        TEXT NOT NULL CHECK (state IN ('queued', 'active', 'done', 'error')),
    error        TEXT,
    rule_id      TEXT,
    requested_at INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS download_state ON download (state);
CREATE INDEX IF NOT EXISTS download_rule ON download (rule_id);
