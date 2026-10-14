-- DB9: the download ledger (CACHE-8). `media.db` owns the artifacts on disk
-- (`media`) and the container selections (`media_rule`); this database owns
-- the per-song intent and progress: what the user asked for, what the fetcher
-- is doing, what failed and why. Disposable: queued rows can be re-derived
-- from the rules, done rows from media.db.

CREATE TABLE download (
    song_id      TEXT PRIMARY KEY,
    state        TEXT NOT NULL CHECK (state IN ('queued', 'active', 'done', 'error')),
    error        TEXT,
    rule_id      TEXT,
    requested_at INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX download_state ON download (state);
CREATE INDEX download_rule ON download (rule_id);
