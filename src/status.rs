// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! DB7 `statussave.db`: the tiny session row P-6 resumes from (roadmap
//! "Session status" design).
//!
//! This is the storage layer only: one row, last write wins, account
//! scoped. The backend owns the dirty/debounce policy and applies the
//! values after `login_succeeded()`; nothing here touches the player.

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

/// One session snapshot; `None` ids simply mean "nothing to restore".
/// `queue` is a JSON array of song ids (empty string = none), `queue_index`
/// points at `song_id` inside it and `position` is the saved playhead in
/// seconds.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SessionStatus {
    pub server_url: String,
    pub username: String,
    pub album_id: Option<String>,
    pub song_id: Option<String>,
    pub queue: String,
    pub queue_index: i64,
    pub position: f64,
    pub volume: f64,
    pub muted: bool,
    pub updated_at: i64,
}

/// Upserts the single row (last write wins, whatever account wrote before).
pub async fn save(pool: &SqlitePool, status: &SessionStatus) -> Result<()> {
    sqlx::query(
        "INSERT INTO session_status
             (id, server_url, username, album_id, song_id, queue, queue_index,
              position, volume, muted, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
             server_url  = excluded.server_url,
             username    = excluded.username,
             album_id    = excluded.album_id,
             song_id     = excluded.song_id,
             queue       = excluded.queue,
             queue_index = excluded.queue_index,
             position    = excluded.position,
             volume      = excluded.volume,
             muted       = excluded.muted,
             updated_at  = excluded.updated_at",
    )
    .bind(&status.server_url)
    .bind(&status.username)
    .bind(&status.album_id)
    .bind(&status.song_id)
    .bind(&status.queue)
    .bind(status.queue_index)
    .bind(status.position)
    .bind(status.volume)
    .bind(status.muted)
    .bind(status.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Loads the row; `None` when nothing was ever saved (first launch).
pub async fn load(pool: &SqlitePool) -> Result<Option<SessionStatus>> {
    let row = sqlx::query_as::<_, SessionStatus>(
        "SELECT server_url, username, album_id, song_id, queue, queue_index,
                position, volume, muted, updated_at
         FROM session_status WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
