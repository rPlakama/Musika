-- ARC-42: cached artist biography + external info (Navidrome's `getArtistInfo2`
-- → `getArtistInfo` fallback). Disposable: refetched on demand and purged when
-- the artist leaves the library (same policy as the other caches).

CREATE TABLE artist_info (
    artist_id  TEXT PRIMARY KEY,
    biography  TEXT NOT NULL DEFAULT '',
    url        TEXT NOT NULL DEFAULT '',
    fetched_at TEXT NOT NULL
);
