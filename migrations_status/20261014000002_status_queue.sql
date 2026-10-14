-- DB7 S3: the queue snapshot and playback position complete the session
-- resume. `queue` is a JSON array of song ids in playback order;
-- `queue_index` points at the current track inside it. Unknown ids are
-- dropped at restore time (queue::resume_payload), like every other index
-- lookup.

ALTER TABLE session_status ADD COLUMN queue TEXT NOT NULL DEFAULT '';
ALTER TABLE session_status ADD COLUMN queue_index INTEGER NOT NULL DEFAULT 0;
ALTER TABLE session_status ADD COLUMN position REAL NOT NULL DEFAULT 0;
