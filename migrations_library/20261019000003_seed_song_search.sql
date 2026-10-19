-- Seed the restored FTS table so the hybrid fill works before the next sync.
-- Separate from the restore migration: that file may already be recorded in
-- `_sqlx_migrations`, so its checksum must not change.
DELETE FROM song_search;
INSERT INTO song_search (title, artist, album, duration, song_id, play_count)
SELECT title,
       COALESCE(artist, ''),
       COALESCE(album, ''),
       CAST(duration AS TEXT),
       id,
       COALESCE(play_count, 0)
FROM songs;
