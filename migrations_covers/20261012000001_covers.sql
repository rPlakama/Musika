-- DB3: cover art cache index. The image blobs themselves live next to the
-- databases under `covers/`; this table only maps a coverArt id to its file
-- and the validators used for conditional revalidation.

CREATE TABLE covers (
    id            TEXT PRIMARY KEY,
    file          TEXT NOT NULL,
    hash          TEXT,
    content_type  TEXT,
    size          INTEGER,
    etag          TEXT,
    last_modified TEXT,
    fetched_at    TEXT NOT NULL
);
