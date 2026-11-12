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

//! Every unit test, split by production module.
//!
//! The production modules are left free of `#[cfg(test)] mod tests` blocks;
//! each module in `test/` imports its own production module and keeps that
//! module's original test helpers and cases.

#![expect(
    unused_imports,
    reason = "the shared test prelude is imported once for every module under test/"
)]

mod common;

use sqlx::sqlite::SqlitePool;

async fn memory_pool(migrations: impl Into<Option<sqlx::migrate::Migrator>>) -> SqlitePool {
    use std::str::FromStr;

    let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    if let Some(migrations) = migrations.into() {
        migrations.run(&pool).await.unwrap();
    }
    pool
}

mod api;

mod radio;

mod artist_user;

mod browse;

mod cache;

mod colors;

mod colors_scheme;

mod covers;

mod crypto;

mod daemon_ipc;

mod db;

mod download;

#[cfg(feature = "gpuui")]
mod download_command;

mod fanout;

mod hex;

mod http_client;

mod index;

mod command_registry;
mod ipc;

mod ipc_protocol;

mod ui_view;

mod library;

mod library_paths;

mod lyrics;

mod media;

mod model;

mod mpris;

mod playback;

mod playlists;

mod queue;

mod search;

mod search_index;

mod status;

mod benchmark;

mod scroll_memory;

mod rows;

/// Sync reconciliation (ARC: library changes vs removals/additions).
mod sync_reconcile;

/// Server list fields may be a single object or an array; an empty array must
/// stay empty (it used to deserialize into one default row with an empty id).
mod one_or_many_lists;
