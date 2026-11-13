-- Local radio (similar-songs expansion without a server round-trip).
--
-- `artist_info.similar` caches the `getArtistInfo2` similar-artist list as a
-- JSON array of `{id, name}` entries. The row is fetched for the artist page
-- biography anyway, so the graph is free to keep; the local recommendation
-- engine prefers it over metadata heuristics and invalidates it with the
-- biography (artist change / provider update / artist leaves the library).
ALTER TABLE artist_info ADD COLUMN similar TEXT NOT NULL DEFAULT '';

-- The similar-artist graph and the biography are refreshed by different
-- callers (radio graph vs artist page), so they keep separate timestamps:
-- a radio-only graph fill must not make an empty biography look fresh.
ALTER TABLE artist_info ADD COLUMN similar_fetched_at TEXT NOT NULL DEFAULT '';

-- Song candidate queries scan genre/year; artist_id and album_id are already
-- indexed, but the local expansion also filters by tag and era.
CREATE INDEX IF NOT EXISTS idx_songs_genre ON songs (genre);
CREATE INDEX IF NOT EXISTS idx_songs_year ON songs (year);
