-- Discriminate the server implementation (from the ping envelope's `type`)
-- so the UI can pick the matching badge icon. Empty = unknown/generic.
ALTER TABLE users ADD COLUMN server_type TEXT NOT NULL DEFAULT '';
