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

/// Rows per `IN (...)` chunk for cross-database and by-id reads.
pub const IN_BATCH: usize = 500;

/// Rows per `INSERT` batch in [`insert_many`].
pub const INSERT_BATCH: usize = 32;

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

/// Decodes a QML payload, falling back to the type default and a debug log.
pub(crate) fn parse_or_default<T: serde::de::DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_else(|err| {
        crate::debug_log!("payload: could not decode: {err}");
        T::default()
    })
}

/// Deletes rows matched by one static `DELETE` statement and one bound key.
pub async fn delete_by<'e, E>(executor: E, sql: &'static str, key: &str) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(sql).bind(key).execute(executor).await?;
    Ok(())
}

/// Appends a comma-separated `?` list and the closing parenthesis. Generic
/// over owned and borrowed ids, so scanned pages do not allocate just to bind.
pub fn push_in_list<S: AsRef<str>>(builder: &mut sqlx::QueryBuilder<sqlx::Sqlite>, ids: &[S]) {
    let mut separated = builder.separated(", ");
    for id in ids {
        separated.push_bind(id.as_ref());
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
    for chunk in rows.chunks(INSERT_BATCH) {
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
    for chunk in ids.chunks(IN_BATCH) {
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
