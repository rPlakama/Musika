-- UI-60: per-album user rating (Subsonic `setRating`, 0 = unrated).
ALTER TABLE albums ADD COLUMN rating INTEGER NOT NULL DEFAULT 0;
