-- Recent order: the provider's `created` timestamp (Subsonic `newest`).
ALTER TABLE albums ADD COLUMN created TEXT;
ALTER TABLE songs  ADD COLUMN created TEXT;

CREATE INDEX IF NOT EXISTS idx_albums_created ON albums (created);
CREATE INDEX IF NOT EXISTS idx_songs_created  ON songs  (created);
