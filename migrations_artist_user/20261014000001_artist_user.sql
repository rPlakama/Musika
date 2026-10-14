-- DB8: per-artist user metadata (ARC-42) — the local index of the user's
-- `star` / `setRating` state. Account-scoped and disposable, like every other
-- cache: losing it only means the next open re-reads the server.

CREATE TABLE artist_user (
    artist_id  TEXT PRIMARY KEY,
    starred    INTEGER NOT NULL DEFAULT 0,
    rating     INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
    updated_at TEXT NOT NULL
);

-- Favourites need a fast "give me every starred artist" scan.
CREATE INDEX artist_user_starred ON artist_user (starred);
