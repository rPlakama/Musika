-- PATH-6: the queue publish reads the starred subset on every row change;
-- a partial index keeps that an index-only scan of the favourites.
CREATE INDEX IF NOT EXISTS idx_songs_starred ON songs (id) WHERE starred IS NOT NULL;
