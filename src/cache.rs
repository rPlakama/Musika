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

//! Shared maintenance scaffolding for the derived caches (ARC-36).
//!
//! The cover/lyric/color stores each hand-rolled the same "drop rows whose
//! owner disappeared" loop. The domain SQL stays in the domain modules; only
//! the diff-and-delete scaffold lives here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use sqlx::AssertSqlSafe;
use sqlx::sqlite::SqlitePool;

/// Makes concurrent writers of the same target use distinct temp files.
static TMP_GEN: AtomicU64 = AtomicU64::new(0);

pub fn write_atomic(dir: &Path, file: &str, bytes: &[u8]) -> Result<PathBuf> {
    let final_path = dir.join(file);
    let tmp_path = dir.join(format!(
        "{file}.{}.{}.tmp",
        std::process::id(),
        TMP_GEN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp_path, bytes)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    if let Err(err) = std::fs::rename(&tmp_path, &final_path) {
        // FIX-10: a failed replace must not litter the cache directory.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err).with_context(|| format!("failed to replace {}", final_path.display()));
    }
    Ok(final_path)
}

/// Deletes rows matched by one static `DELETE` statement and one bound key.
pub async fn delete_by<'e, E>(executor: E, sql: &'static str, key: &str) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(sql).bind(key).execute(executor).await?;
    Ok(())
}

/// Appends a comma-separated `?` list and the closing parenthesis.
pub fn push_in_list(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, ids: &[String]) {
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
}

/// Borrowed twin of [`push_in_list`] for callers whose ids already exist as
/// `&str` (a scanned page) and would otherwise allocate one `String` per id
/// just to bind it.
pub fn push_in_list_refs(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, ids: &[&str]) {
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(*id);
    }
    separated.push_unseparated(")");
}

/// Deletes rows matched by an `... IN (` prefix and the bound ids.
pub async fn delete_in<'e, E>(executor: E, sql: &'static str, ids: &[String]) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    if ids.is_empty() {
        return Ok(());
    }
    let mut builder = sqlx::QueryBuilder::new(sql);
    push_in_list(&mut builder, ids);
    builder.build().execute(executor).await?;
    Ok(())
}

/// Inserts `rows` under a static `INSERT ...` prefix, in bounded batches.
pub async fn insert_many<T, F>(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    sql: &'static str,
    rows: &[T],
    mut push_tuple: F,
) -> Result<()>
where
    F: FnMut(sqlx::query_builder::Separated<'_, sqlx::Sqlite, &'static str>, &T),
{
    for chunk in rows.chunks(32) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(sql);
        query.push_values(chunk, &mut push_tuple);
        query.build().execute(&mut **tx).await?;
    }
    Ok(())
}

/// Deletes rows from `derived` whose `key_col` is absent from `owner_ids`.
///
/// `table`/`key_col` are compile-time constants supplied by the caller (never
/// user input); the ids themselves are bound parameters. Returns the number of
/// removed rows.
pub async fn purge_missing(
    derived: &SqlitePool,
    table: &'static str,
    key_col: &'static str,
    owner_ids: &HashSet<String>,
) -> Result<usize> {
    let select = format!("SELECT {key_col} FROM {table}");
    let ids: Vec<String> = sqlx::query_scalar::<_, String>(AssertSqlSafe(select))
        .fetch_all(derived)
        .await?;
    delete_missing(derived, table, key_col, &ids, owner_ids).await
}

/// Twin of [`purge_missing`] that queries the owner table for **only the
/// derived ids** instead of materializing every owner.
///
/// The `songs` table can be far larger than the lyric/download/media cache, so
/// loading all song ids just to answer "does this cached id still exist?" was
/// the bigger allocation. Here the derived ids are collected (needed anyway),
/// then the owner pool is asked about them in 500-row chunks.
///
/// The table/column names are compile-time constants supplied by the caller;
/// ids are always bound.
pub async fn purge_missing_against(
    derived: &SqlitePool,
    table: &'static str,
    key_col: &'static str,
    owners: &SqlitePool,
    owner_table: &'static str,
    owner_col: &'static str,
) -> Result<usize> {
    let select = format!("SELECT {key_col} FROM {table}");
    let ids: Vec<String> = sqlx::query_scalar::<_, String>(AssertSqlSafe(select))
        .fetch_all(derived)
        .await?;
    if ids.is_empty() {
        return Ok(0);
    }

    let mut owner_ids: HashSet<String> = HashSet::with_capacity(ids.len());
    for chunk in ids.chunks(500) {
        let mut builder = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "SELECT {owner_col} FROM {owner_table} WHERE {owner_col} IN ("
        ));
        push_in_list(&mut builder, chunk);
        let found: Vec<String> = builder.build_query_scalar().fetch_all(owners).await?;
        owner_ids.extend(found);
    }
    delete_missing(derived, table, key_col, &ids, &owner_ids).await
}

async fn delete_missing(
    derived: &SqlitePool,
    table: &'static str,
    key_col: &'static str,
    ids: &[String],
    owner_ids: &HashSet<String>,
) -> Result<usize> {
    let stale: Vec<&String> = ids.iter().filter(|id| !owner_ids.contains(*id)).collect();
    if stale.is_empty() {
        return Ok(0);
    }

    let delete = format!("DELETE FROM {table} WHERE {key_col} = ?1");
    let mut tx = derived.begin().await?;
    for id in &stale {
        sqlx::query(AssertSqlSafe(delete.clone()))
            .bind(*id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(stale.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        crate::sqlite::memory_pool(None).await
    }

    #[tokio::test]
    async fn removes_only_rows_missing_from_the_owner_set() {
        let derived = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&derived)
            .await
            .unwrap();
        for id in ["keep", "gone", "also-gone"] {
            sqlx::query("INSERT INTO cache (id) VALUES (?1)")
                .bind(id)
                .execute(&derived)
                .await
                .unwrap();
        }

        let owners: HashSet<String> = ["keep".to_string()].into_iter().collect();
        assert_eq!(
            purge_missing(&derived, "cache", "id", &owners)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            purge_missing(&derived, "cache", "id", &owners)
                .await
                .unwrap(),
            0
        );
    }

    /// The owner-pool variant asks only about the derived ids; an empty cache
    /// must not query the owner table at all (it can be absent).
    #[tokio::test]
    async fn purge_missing_against_checks_only_the_derived_ids() {
        let derived = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&derived)
            .await
            .unwrap();
        for id in ["keep", "gone", "also-gone"] {
            sqlx::query("INSERT INTO cache (id) VALUES (?1)")
                .bind(id)
                .execute(&derived)
                .await
                .unwrap();
        }
        let owners = pool().await;
        sqlx::query("CREATE TABLE owners (id TEXT PRIMARY KEY)")
            .execute(&owners)
            .await
            .unwrap();
        sqlx::query("INSERT INTO owners (id) VALUES ('keep')")
            .execute(&owners)
            .await
            .unwrap();

        assert_eq!(
            purge_missing_against(&derived, "cache", "id", &owners, "owners", "id")
                .await
                .unwrap(),
            2
        );

        let empty = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&empty)
            .await
            .unwrap();
        let no_owner_table = pool().await;
        assert_eq!(
            purge_missing_against(&empty, "cache", "id", &no_owner_table, "owners", "id")
                .await
                .unwrap(),
            0
        );
    }

    /// FIX-10: a replace that cannot rename (the destination is a directory)
    /// leaves no `*.tmp` behind.
    #[test]
    fn a_failed_replace_removes_the_temp_file() {
        let dir = std::env::temp_dir().join(format!("musika-atomic-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("cover.jpg")).expect("blocking dir");
        assert!(write_atomic(&dir, "cover.jpg", b"bytes").is_err());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("temp dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be cleaned up");
        let _ = std::fs::remove_dir_all(dir);
    }
}
