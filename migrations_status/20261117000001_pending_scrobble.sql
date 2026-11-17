-- SCROBBLE-1: completed plays that could not be submitted yet (offline or a
-- transient server failure). The server's play count is the source of truth;
-- this queue only retries the submission and is flushed by the health probe.
--
-- `played_at` is the Unix second at which the threshold was reached; the
-- Subsonic `time` parameter is sent in milliseconds.

CREATE TABLE pending_scrobble (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    song_id    TEXT    NOT NULL,
    played_at  INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    UNIQUE(song_id, played_at)
);

CREATE INDEX pending_scrobble_created ON pending_scrobble (created_at);
