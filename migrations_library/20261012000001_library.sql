-- DB2a: library metadata. Disposable, rebuilt from the Subsonic API.

CREATE TABLE artists (
    id            TEXT PRIMARY KEY,
    name          TEXT    NOT NULL,
    album_count   INTEGER NOT NULL DEFAULT 0,
    cover_art     TEXT,
    image_url     TEXT,
    starred       TEXT
);

CREATE TABLE albums (
    id          TEXT PRIMARY KEY,
    artist_id   TEXT,
    name        TEXT    NOT NULL,
    artist      TEXT,
    year        INTEGER,
    genre       TEXT,
    song_count  INTEGER NOT NULL DEFAULT 0,
    duration    INTEGER NOT NULL DEFAULT 0,
    cover_art   TEXT,
    starred     TEXT
);

CREATE TABLE songs (
    id          TEXT PRIMARY KEY,
    album_id    TEXT,
    artist_id   TEXT,
    title       TEXT    NOT NULL,
    album       TEXT,
    artist      TEXT,
    track       INTEGER,
    disc_number INTEGER,
    year        INTEGER,
    genre       TEXT,
    duration    INTEGER,
    bit_rate    INTEGER,
    suffix      TEXT,
    size        INTEGER,
    path        TEXT,
    cover_art   TEXT,
    play_count  INTEGER,
    starred     TEXT
);

CREATE INDEX idx_albums_artist ON albums (artist_id);
CREATE INDEX idx_songs_album   ON songs (album_id);
CREATE INDEX idx_songs_artist  ON songs (artist_id);

-- Sync bookkeeping (`last_modified`, `last_sync`).
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
