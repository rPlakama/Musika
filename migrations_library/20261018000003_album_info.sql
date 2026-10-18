-- PATH-8: cached album descriptions, served before the server refresh.
CREATE TABLE IF NOT EXISTS album_info (
    album_id   TEXT PRIMARY KEY,
    notes      TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL
);
