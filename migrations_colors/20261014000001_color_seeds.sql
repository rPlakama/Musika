-- THM-6 R1/R3: seed cache + palette identity.
--
-- The seed is the expensive step (decode, downscale, quantize, score); it is
-- cached per cover content and source mode with a small LRU so variant, spec or
-- contrast changes do not re-quantize the image. `role_hash` lets the UI skip
-- re-applying a palette identical to the one on screen.

ALTER TABLE cover_color ADD COLUMN role_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE cover_color ADD COLUMN colorfulness REAL NOT NULL DEFAULT 0;

CREATE TABLE color_seed (
    cover_hash   TEXT NOT NULL,
    mode         TEXT NOT NULL,            -- dominant | colorful
    engine       TEXT NOT NULL,            -- extraction pipeline version
    seed         INTEGER NOT NULL,         -- ARGB of the scored source color
    colorfulness REAL NOT NULL DEFAULT 0,  -- drives the `smart` variant
    used         INTEGER NOT NULL,
    PRIMARY KEY (cover_hash, mode, engine)
);

CREATE INDEX idx_color_seed_used ON color_seed (used);
