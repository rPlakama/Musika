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

//! Subsonic scrobbling.
//!
//! The server owns play counts. Playback reports a now-playing notification
//! on start and one completed submission once a track has been heard for
//! `min(4 minutes, half its duration)`. A failed/offline submission is queued
//! in `status.db` and retried by the health probe, oldest first. The local
//! count is not incremented: the next library sync receives the server's
//! authoritative value.

use std::sync::OnceLock;

use anyhow::{Context, Result};
use sqlx::sqlite::SqlitePool;

use crate::db::Dbs;

/// Maximum retry attempts before a failing scrobble row is dropped as a
/// poison pill so one bad/deleted track cannot wedge the queue forever.
const MAX_ATTEMPTS: i64 = 5;

/// Guards [`flush`] against overlapping invocations from concurrent health
/// probes or reconnect ticks.
static FLUSH_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

/// Last.fm-style threshold: four minutes or half the track, whichever comes
/// first. Unknown durations wait four minutes; `0` would submit instantly.
pub fn threshold_secs(duration: f64) -> f64 {
    if duration > 0.0 {
        (duration * 0.5).clamp(1.0, 240.0)
    } else {
        240.0
    }
}

/// Best-effort now-playing notification. Offline is a silent no-op.
pub async fn now_playing(dbs: &Dbs, song_id: &str) {
    if song_id.is_empty() || !crate::session::online() {
        return;
    }
    let Ok(client) = crate::session::client(dbs).await else {
        return;
    };
    if let Err(error) = client.scrobble(song_id, false, None).await {
        crate::debug_log!("scrobble: now-playing {song_id} failed: {error:#}");
    }
}

/// Submits one completed play; queues it for retry if the server is not
/// reachable (or answers with an error).
pub async fn submit(dbs: &Dbs, song_id: &str, played_at: i64) {
    if song_id.is_empty() {
        return;
    }
    if !crate::session::online() {
        queue(dbs, song_id, played_at).await;
        return;
    }
    let Ok(client) = crate::session::client(dbs).await else {
        queue(dbs, song_id, played_at).await;
        return;
    };
    match client
        .scrobble(song_id, true, Some(played_at.saturating_mul(1000)))
        .await
    {
        Ok(()) => crate::debug_log!("scrobble: submitted {song_id}"),
        Err(error) => {
            crate::debug_log!("scrobble: {song_id} queued after failure: {error:#}");
            queue(dbs, song_id, played_at).await;
        }
    }
}

async fn queue(dbs: &Dbs, song_id: &str, played_at: i64) {
    let result = sqlx::query(
        "INSERT INTO pending_scrobble (song_id, played_at, created_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(song_id, played_at) DO NOTHING",
    )
    .bind(song_id)
    .bind(played_at)
    .bind(crate::consts::now_secs())
    .execute(&dbs.status)
    .await;
    if let Err(error) = result {
        eprintln!("scrobble: could not queue {song_id}: {error:#}");
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PendingScrobble {
    id: i64,
    song_id: String,
    played_at: i64,
    attempts: i64,
}

async fn fetch_pending(pool: &SqlitePool) -> Result<Vec<PendingScrobble>> {
    let rows = sqlx::query_as::<_, PendingScrobble>(
        "SELECT id, song_id, played_at, attempts FROM pending_scrobble
         ORDER BY created_at, id LIMIT 50",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM pending_scrobble WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn record_failure(pool: &SqlitePool, id: i64, message: &str) -> Result<()> {
    sqlx::query(
        "UPDATE pending_scrobble
         SET attempts = attempts + 1, last_error = ?2
         WHERE id = ?1",
    )
    .bind(id)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

fn is_permanent_scrobble_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("data not found")
        || lower.contains("requested data was not found")
        || lower.contains("song not found")
        || lower.contains("item not found")
        || lower.contains("code 70")
}

async fn flush_rows<F, Fut>(
    pool: &SqlitePool,
    rows: Vec<PendingScrobble>,
    mut submit_one: F,
) -> Result<usize>
where
    F: FnMut(String, i64) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let mut sent = 0usize;
    for entry in rows {
        match submit_one(entry.song_id.clone(), entry.played_at).await {
            Ok(()) => {
                delete(pool, entry.id).await?;
                sent += 1;
            }
            Err(error) => {
                let message = format!("{error:#}");
                record_failure(pool, entry.id, &message).await?;
                if entry.attempts + 1 >= MAX_ATTEMPTS || is_permanent_scrobble_error(&message) {
                    crate::debug_log!(
                        "scrobble: dropping poisoned row {} (song {}, attempts {}): {message}",
                        entry.id,
                        entry.song_id,
                        entry.attempts + 1
                    );
                    delete(pool, entry.id).await?;
                    continue;
                }
                break;
            }
        }
    }
    Ok(sent)
}

/// Flushes at most 50 queued submissions. Stops at the first transient failure
/// so the next probe retries in order instead of hammering a server that is
/// down, while evicting rows that exceed [`MAX_ATTEMPTS`] or fail with a
/// permanent data-not-found error.
pub async fn flush(dbs: &Dbs) -> Result<usize> {
    let Ok(_guard) = FLUSH_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .try_lock()
    else {
        return Ok(0);
    };
    let rows = fetch_pending(&dbs.status).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    if !crate::session::online() {
        return Ok(0);
    }
    let client = crate::session::client(dbs).await?;
    flush_rows(&dbs.status, rows, |song_id, played_at| {
        let client = client.clone();
        async move {
            client
                .scrobble(&song_id, true, Some(played_at.saturating_mul(1000)))
                .await
        }
    })
    .await
}

/// Number of queued submissions (diagnostics/tests).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the unit tests call this helper")
)]
pub async fn pending_count(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM pending_scrobble")
        .fetch_one(pool)
        .await
        .context("count pending scrobbles")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    async fn test_status_pool() -> SqlitePool {
        let options =
            sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").expect("sqlite url");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("memory pool");
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .expect("library migrations");
        pool
    }

    fn test_dbs(pool: &SqlitePool) -> Dbs {
        Dbs {
            credentials: pool.clone(),
            library: pool.clone(),
            sync: pool.clone(),
            search: pool.clone(),
            covers: pool.clone(),
            media: pool.clone(),
            lyrics: pool.clone(),
            colors: pool.clone(),
            status: pool.clone(),
            artist_user: pool.clone(),
            download: pool.clone(),
            playlist: pool.clone(),
        }
    }

    #[tokio::test]
    async fn poison_pill_evicts_after_max_attempts_and_flushes_subsequent_rows() {
        let pool = test_status_pool().await;
        let dbs = test_dbs(&pool);

        queue(&dbs, "poison", 1_700_000_001).await;
        queue(&dbs, "good", 1_700_000_002).await;
        assert_eq!(pending_count(&pool).await.expect("count"), 2);

        // First MAX_ATTEMPTS - 1 transient failures keep the row and stop the pass.
        for expected_attempts in 1..MAX_ATTEMPTS {
            let rows = fetch_pending(&pool).await.expect("rows");
            let sent = flush_rows(&pool, rows, |song_id, _| async move {
                if song_id == "poison" {
                    anyhow::bail!("transient socket timeout");
                }
                Ok(())
            })
            .await
            .expect("flush_rows");
            assert_eq!(sent, 0);
            assert_eq!(pending_count(&pool).await.expect("count"), 2);
            let rows = fetch_pending(&pool).await.expect("rows");
            assert_eq!(rows[0].song_id, "poison");
            assert_eq!(rows[0].attempts, expected_attempts);
        }

        // The MAX_ATTEMPTS-th failure evicts "poison" and continues to flush "good".
        let rows = fetch_pending(&pool).await.expect("rows");
        let sent = flush_rows(&pool, rows, |song_id, _| async move {
            if song_id == "poison" {
                anyhow::bail!("transient socket timeout");
            }
            Ok(())
        })
        .await
        .expect("flush_rows");
        assert_eq!(sent, 1);
        assert_eq!(pending_count(&pool).await.expect("count"), 0);
    }

    #[tokio::test]
    async fn permanent_not_found_error_evicts_immediately_and_continues() {
        let pool = test_status_pool().await;
        let dbs = test_dbs(&pool);

        queue(&dbs, "deleted-song", 1_700_000_010).await;
        queue(&dbs, "healthy-song", 1_700_000_020).await;

        let rows = fetch_pending(&pool).await.expect("rows");
        let sent = flush_rows(&pool, rows, |song_id, _| async move {
            if song_id == "deleted-song" {
                anyhow::bail!("Server rejected the request (code 70): Data not found");
            }
            Ok(())
        })
        .await
        .expect("flush_rows");

        assert_eq!(sent, 1);
        assert_eq!(pending_count(&pool).await.expect("count"), 0);
    }

    #[tokio::test]
    async fn flush_lock_prevents_overlapping_flushes() {
        let pool = test_status_pool().await;
        let dbs = test_dbs(&pool);
        queue(&dbs, "s1", 1_700_000_100).await;

        let lock = FLUSH_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
        let held = lock.lock().await;

        // While the lock is held by an in-flight flush, a concurrent flush
        // returns Ok(0) immediately without blocking or double-submitting.
        let concurrent = flush(&dbs).await.expect("concurrent flush");
        assert_eq!(concurrent, 0);
        assert_eq!(pending_count(&pool).await.expect("count"), 1);
        drop(held);
    }
}
