-- ARC-memory: squash the account/library-side databases into library.db.
-- credentials, search, artist_user, statussave, colors and playlists now live
-- beside the metadata tables so one pool owns the whole library-side state.

CREATE TABLE IF NOT EXISTS users (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    username    TEXT NOT NULL,
    password    TEXT NOT NULL,
    serverURL   TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT(datetime('now')),
    server_type TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS artist_user (
    artist_id  TEXT PRIMARY KEY,
    starred    INTEGER NOT NULL DEFAULT 0,
    rating     INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS artist_user_starred ON artist_user (starred);

CREATE TABLE IF NOT EXISTS session_status (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    server_url  TEXT    NOT NULL,
    username    TEXT    NOT NULL,
    album_id    TEXT,
    song_id     TEXT,
    volume      REAL    NOT NULL DEFAULT 70.0,
    muted       INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL,
    queue       TEXT    NOT NULL DEFAULT '',
    queue_index INTEGER NOT NULL DEFAULT 0,
    position    REAL    NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS cover_color (
    cover_art    TEXT PRIMARY KEY,
    cover_hash   TEXT NOT NULL,
    scheme       TEXT NOT NULL,
    seed         INTEGER NOT NULL,
    dark         TEXT NOT NULL,
    generator    TEXT NOT NULL,
    generated_at INTEGER NOT NULL,
    role_hash    TEXT NOT NULL DEFAULT '',
    colorfulness REAL NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS colors_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS color_seed (
    cover_hash   TEXT NOT NULL,
    mode         TEXT NOT NULL,
    engine       TEXT NOT NULL,
    seed         INTEGER NOT NULL,
    colorfulness REAL NOT NULL DEFAULT 0,
    used         INTEGER NOT NULL,
    PRIMARY KEY (cover_hash, mode, engine)
);
CREATE INDEX IF NOT EXISTS idx_color_seed_used ON color_seed (used);

CREATE TABLE IF NOT EXISTS playlist (
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

CREATE TABLE IF NOT EXISTS playlist_song (
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
CREATE INDEX IF NOT EXISTS playlist_song_song ON playlist_song (song_id);

CREATE TABLE IF NOT EXISTS playlist_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS song_search USING fts5 (
    title,
    artist,
    album,
    duration UNINDEXED,
    song_id UNINDEXED,
    play_count UNINDEXED,
    tokenize = 'unicode61'
);
