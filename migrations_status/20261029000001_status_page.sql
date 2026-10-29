-- P-6: remember the last page kind/id, not just an album. Existing rows keep
-- their album_id and fall back to it at restore time.
ALTER TABLE session_status ADD COLUMN page_kind TEXT NOT NULL DEFAULT '';
ALTER TABLE session_status ADD COLUMN page_id   TEXT NOT NULL DEFAULT '';
