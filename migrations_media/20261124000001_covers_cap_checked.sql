-- Track the COVER_MAX_EDGE threshold at which each cached cover blob was last
-- verified (0 = never checked). Avoids re-reading/re-probing every >2 MiB cover
-- on every daemon startup.
ALTER TABLE covers ADD COLUMN cap_checked INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS covers_cap_check
    ON covers (cap_checked, size DESC, id ASC);
