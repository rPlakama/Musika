-- DB9: the playlist index (ARC-44). Disposable cache over two sources —
-- Subsonic playlists and the local `playlists/*.m3u` files; the server and
-- the files stay the source of truth. `playlist_song` is denormalized so a
-- playlist renders even when a song left library.db (unmatched entries keep
-- `song_id` NULL, render dimmed and skip on play). `path`/`mtime` are the
-- m3u file identity; server rows leave them NULL. Positions are playlist
-- order and duplicates are kept, never deduped.

CREATE TABLE playlist (
    id         TEXT PRIMARY KEY,
    source     TEXT    NOT NULL CHECK (source IN ('subsonic', 'm3u')),
    name       TEXT    NOT NULL,
    comment    TEXT,
    owner      TEXT,
    public     INTEGER,
    song_count INTEGER NOT NULL DEFAULT 0,
    duration   INTEGER NOT NULL DEFAULT 0,
    created    TEXT,
    changed    TEXT,
    cover_art  TEXT,
    path       TEXT,
    mtime      INTEGER,
    fetched_at INTEGER NOT NULL
);

CREATE TABLE playlist_song (
    playlist_id TEXT    NOT NULL REFERENCES playlist (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    song_id     TEXT,
    title       TEXT    NOT NULL,
    artist      TEXT,
    album       TEXT,
    duration    INTEGER,
    cover_art   TEXT,
    PRIMARY KEY (playlist_id, position)
);

CREATE INDEX playlist_song_song ON playlist_song (song_id);

CREATE TABLE playlist_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
