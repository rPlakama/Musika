-- DB4: offline audio cache index (CACHE-1). Disposable: losing it only means
-- re-downloading. Blobs live in ~/.config/musika/media/ (permanent/rule) and
-- ~/.cache/musika/media/ (playback/rolling, purgeable).

CREATE TABLE media (
    song_id     TEXT PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('playback', 'rolling', 'permanent', 'rule')),
    file        TEXT NOT NULL,
    size        INTEGER NOT NULL,
    quality     TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    rule_id     TEXT,
    last_used   INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE INDEX media_last_used ON media (last_used);
CREATE INDEX media_rule_id ON media (rule_id);

-- Auto-download containers (CACHE-5): artist / album / playlist / genre /
-- favourite. Materialized by the sync-time rule pass, never by this layer.
CREATE TABLE media_rule (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('artist', 'album', 'playlist', 'genre', 'favorite')),
    ref_id     TEXT,
    created_at INTEGER NOT NULL
);

-- Tier caps and bookkeeping (CACHE-3/4).
CREATE TABLE media_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
