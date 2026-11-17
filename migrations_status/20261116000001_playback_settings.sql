-- PERSIST-1: playback behaviour that must survive a restart. Volume/mute and
-- the last queue live in session_status; this key/value table owns the rest
-- (repeat/shuffle, crossfade, normalization, EQ profile, output device,
-- stream quality and cache). Values are JSON strings so a setting added by a
-- later phase does not need a schema migration.

CREATE TABLE playback_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
