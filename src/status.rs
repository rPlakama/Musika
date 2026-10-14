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
//! This is the S0 storage layer only: one row, last write wins, account
//! scoped. The backend (S1+) owns the dirty/debounce policy and applies the
//! values after `login_succeeded()`; nothing here touches the player.

use anyhow::Result;
use sqlx::sqlite::SqlitePool;

/// One session snapshot; `None` ids simply mean "nothing to restore".
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct SessionStatus {
    pub server_url: String,
    pub username: String,
    pub album_id: Option<String>,
    pub song_id: Option<String>,
    pub volume: f64,
    pub muted: bool,
    pub updated_at: i64,
}

/// Upserts the single row (last write wins, whatever account wrote before).
pub async fn save(pool: &SqlitePool, status: &SessionStatus) -> Result<()> {
    sqlx::query(
        "INSERT INTO session_status
             (id, server_url, username, album_id, song_id, volume, muted, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             server_url = excluded.server_url,
             username   = excluded.username,
             album_id   = excluded.album_id,
             song_id    = excluded.song_id,
             volume     = excluded.volume,
             muted      = excluded.muted,
             updated_at = excluded.updated_at",
    )
    .bind(&status.server_url)
    .bind(&status.username)
    .bind(&status.album_id)
    .bind(&status.song_id)
    .bind(status.volume)
    .bind(status.muted as i64)
    .bind(status.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Loads the row; `None` when nothing was ever saved (first launch).
pub async fn load(pool: &SqlitePool) -> Result<Option<SessionStatus>> {
    let row = sqlx::query_as::<_, SessionStatus>(
        "SELECT server_url, username, album_id, song_id, volume, muted, updated_at
         FROM session_status WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    async fn memory_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations_status")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn round_trips_and_last_write_wins() {
        let pool = memory_pool().await;
        assert!(load(&pool).await.unwrap().is_none());

        let first = SessionStatus {
            server_url: "https://music.example".to_string(),
            username: "demo".to_string(),
            album_id: Some("al1".to_string()),
            song_id: Some("s1".to_string()),
            volume: 42.0,
            muted: true,
            updated_at: 100,
        };
        save(&pool, &first).await.unwrap();
        assert_eq!(load(&pool).await.unwrap().as_ref(), Some(&first));

        // The single row is replaced, never duplicated.
        let second = SessionStatus {
            album_id: None,
            song_id: None,
            volume: 70.0,
            muted: false,
            updated_at: 200,
            ..first.clone()
        };
        save(&pool, &second).await.unwrap();
        let loaded = load(&pool).await.unwrap().unwrap();
        assert_eq!(loaded, second);
        assert_eq!(loaded.album_id, None);
        assert_eq!(loaded.updated_at, 200);
    }
}
