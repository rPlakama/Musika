-- DB6: cover-seeded dynamic color cache (text only, no blob dir).
--
-- Re-fetchable content in its own pool, same multiDB policy as covers/lyrics:
-- the library writer never contends with palette reads. Rows are keyed by
-- cover_art id and revalidated against `covers.hash`; a generator bump or a
-- changed image makes the row stale and it regenerates.

CREATE TABLE cover_color (
    cover_art    TEXT PRIMARY KEY,
    cover_hash   TEXT NOT NULL,    -- covers.hash at generation time
    scheme       TEXT NOT NULL,    -- tonal_spot | vibrant | smart | ...
    seed         INTEGER NOT NULL, -- ARGB of the scored source color
    dark         TEXT NOT NULL,    -- JSON {role: "#RRGGBB"} map
    generator    TEXT NOT NULL,    -- engine + version tag
    generated_at INTEGER NOT NULL
);

CREATE TABLE colors_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
