-- Playback preferences live in status.db in tests, which is the production
-- library file (status is an alias of library). Keep the production schema in
-- step with migrations_status.
CREATE TABLE IF NOT EXISTS playback_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
