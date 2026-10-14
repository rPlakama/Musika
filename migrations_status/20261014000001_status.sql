-- DB7: session status (P-6 / roadmap "Session status"). Single row, last
-- write wins; the row carries the account it was written for, so logging
-- into another account simply overwrites it on the first change.

CREATE TABLE session_status (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    server_url  TEXT    NOT NULL,
    username    TEXT    NOT NULL,
    album_id    TEXT,
    song_id     TEXT,
    volume      REAL    NOT NULL DEFAULT 70.0,
    muted       INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
);
