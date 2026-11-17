-- SCROBBLE-1: completed plays whose submission failed or happened offline.
-- The status pool aliases this library file; this is the one schema owner.
CREATE TABLE IF NOT EXISTS pending_scrobble (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id    TEXT    NOT NULL,
    played_at  INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    UNIQUE(song_id, played_at)
);
CREATE INDEX IF NOT EXISTS pending_scrobble_created ON pending_scrobble (created_at);
