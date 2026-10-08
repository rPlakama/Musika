-- Add migration script here
CREATE TABLE users(id INTEGER PRIMARY KEY AUTOINCREMENT,
username TEXT NOT NULL,
password TEXT NOT NULL,
serverURL TEXT NOT NULL,
created_at TEXT NOT NULL DEFAULT(datetime('now')));
