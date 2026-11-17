-- PERSIST-1: playback preferences. The status pool is an alias of the library
-- file, so this is the single schema declaration for both production and the
-- test fixtures.
CREATE TABLE IF NOT EXISTS playback_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
