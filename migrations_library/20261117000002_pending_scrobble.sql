-- See migrations_status/20261117000001_pending_scrobble.sql: the production
-- status pool is the library file, so the retry queue must exist here too.
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
