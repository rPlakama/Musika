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

//! Library sync writes: the whole-table replace and the index mirror.

use std::collections::HashMap;

use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use sqlx::sqlite::SqlitePool;

use crate::api::{AlbumID3, ArtistID3, Child};

use super::{AlbumSnapshot, ArtistSnapshot, SongSnapshot, SyncChanges};

fn snapshot_hash<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Replaces all library tables in a single transaction and stores the sync
/// bookkeeping values.
///
/// Returns the song/album/artist/cover ids that changed so the caller can
/// refresh the derived caches (covers, lyrics, descriptions) for exactly
/// those rows.
pub async fn replace(
    pool: &SqlitePool,
    artists: &[ArtistID3],
    albums: &[AlbumID3],
    songs: &[Child],
    last_modified: i64,
) -> Result<SyncChanges> {
    let mut tx = pool.begin().await?;

    // MEMORY: hold an `id -> hash` (plus the album cover id, the one field the
    // cover-refresh decision still needs) instead of a full snapshot per row.
    // Albums/artists are few compared with songs, but the same compact shape
    // keeps the diff's peak flat and one row at a time crosses the boundary.
    struct AlbumSeen {
        hash: u64,
        cover_art: Option<String>,
    }
    let mut old_albums: HashMap<String, AlbumSeen> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, AlbumSnapshot>(
            "SELECT id, artist_id, name, artist, year, genre, song_count, duration, cover_art,
                    created
             FROM albums",
        )
        .fetch(&mut *tx);
        while let Some(album) = rows.try_next().await? {
            let hash = snapshot_hash(&album);
            old_albums.insert(
                album.id.clone(),
                AlbumSeen {
                    hash,
                    cover_art: album.cover_art,
                },
            );
        }
    }
    let mut old_artists: HashMap<String, u64> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, ArtistSnapshot>(
            "SELECT id, name, album_count, cover_art, image_url FROM artists",
        )
        .fetch(&mut *tx);
        while let Some(artist) = rows.try_next().await? {
            old_artists.insert(artist.id.clone(), snapshot_hash(&artist));
        }
    }
    // MEMORY: stream the old songs instead of collecting them. Only the hash
    // survives each row, so the peak stays one row plus the compact map.
    let mut old_songs: HashMap<String, u64> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, SongSnapshot>(
            "SELECT id, album_id, artist_id, title, album, artist, track, disc_number, year,
                    genre, duration, bit_rate, suffix, size, path, cover_art, created
             FROM songs",
        )
        .fetch(&mut *tx);
        while let Some(song) = rows.try_next().await? {
            old_songs.insert(song.id.clone(), snapshot_hash(&song));
        }
    }

    // MEMORY: resolve the diff before the destructive write, then drop the old
    // catalog. The insert phase is the peak (new rows + SQLite pages), so the
    // old ids/hashes must not ride along through it.
    let mut changes = SyncChanges::default();
    for song in songs {
        match old_songs.get(&song.id) {
            Some(old) if *old == snapshot_hash(&SongSnapshot::from_child(song)) => {}
            _ => changes.songs.push(song.id.clone()),
        }
    }
    for album in albums {
        let seen = old_albums.get(&album.id);
        if seen.is_some_and(|old| old.hash == snapshot_hash(&AlbumSnapshot::from_album(album))) {
            continue;
        }
        changes.albums.push(album.id.clone());
        let cover_changed = seen.is_none_or(|old| old.cover_art != album.cover_art);
        if cover_changed
            && let Some(cover) = album.cover_art.clone().filter(|cover| !cover.is_empty())
        {
            changes.covers.push(cover);
        }
    }
    for artist in artists {
        let changed = old_artists
            .get(&artist.id)
            .is_none_or(|old| *old != snapshot_hash(&ArtistSnapshot::from_artist(artist)));
        if changed {
            changes.artists.push(artist.id.clone());
        }
    }
    drop(old_songs);
    drop(old_albums);
    drop(old_artists);

    sqlx::query("DELETE FROM songs").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM albums").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM artists").execute(&mut *tx).await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT INTO artists (id, name, album_count, cover_art, image_url, starred) ",
        artists,
        |mut row, artist| {
            row.push_bind(&artist.id)
                .push_bind(&artist.name)
                .push_bind(artist.album_count)
                .push_bind(&artist.cover_art)
                .push_bind(&artist.artist_image_url)
                .push_bind(&artist.starred);
        },
    )
    .await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT INTO albums
             (id, artist_id, name, artist, year, genre, song_count, duration, cover_art, starred,
              rating, created) ",
        albums,
        |mut row, album| {
            row.push_bind(&album.id)
                .push_bind(&album.artist_id)
                .push_bind(&album.name)
                .push_bind(&album.artist)
                .push_bind(album.year)
                .push_bind(&album.genre)
                .push_bind(album.song_count)
                .push_bind(album.duration)
                .push_bind(&album.cover_art)
                .push_bind(&album.starred)
                .push_bind(album.user_rating.unwrap_or(0).clamp(0, 5))
                .push_bind(&album.created);
        },
    )
    .await?;

    crate::cache::insert_many(
        &mut tx,
        "INSERT OR REPLACE INTO songs
             (id, album_id, artist_id, title, album, artist, track, disc_number, year,
              genre, duration, bit_rate, suffix, size, path, cover_art, play_count, starred,
              created) ",
        songs,
        |mut row, song| {
            row.push_bind(&song.id)
                .push_bind(&song.album_id)
                .push_bind(&song.artist_id)
                .push_bind(&song.title)
                .push_bind(&song.album)
                .push_bind(&song.artist)
                .push_bind(song.track)
                .push_bind(song.disc_number)
                .push_bind(song.year)
                .push_bind(&song.genre)
                .push_bind(song.duration)
                .push_bind(song.bit_rate)
                .push_bind(&song.suffix)
                .push_bind(song.size)
                .push_bind(&song.path)
                .push_bind(&song.cover_art)
                .push_bind(song.play_count)
                .push_bind(&song.starred)
                .push_bind(&song.created);
        },
    )
    .await?;

    upsert_meta(&mut *tx, "last_modified", &last_modified.to_string()).await?;
    upsert_meta(
        &mut *tx,
        "last_sync",
        &crate::consts::now_secs().to_string(),
    )
    .await?;

    tx.commit().await?;

    Ok(changes)
}

/// ARC-I-4: the local half of the favourite toggle. The UI reads
/// `starred IS NOT NULL` as "favourited" (UI-56); the exact server timestamp
/// reconciles on the next sync. Songs and albums only — artists live in
/// `artist_user.db` (`artist_user::set_starred`).
pub async fn touch_last_sync(pool: &SqlitePool) -> Result<()> {
    upsert_meta(pool, "last_sync", &crate::consts::now_secs().to_string()).await
}

pub async fn last_modified(pool: &SqlitePool) -> Result<i64> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'last_modified'")
            .fetch_optional(pool)
            .await?;
    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

/// Snapshot shown by the right status bar.
pub async fn write_mirror(pool: &SqlitePool) -> Result<()> {
    use std::fmt::Write as _;
    use tokio::io::AsyncWriteExt as _;

    let path = crate::db::mirror_path()?;
    crate::db::ensure_parent_dir(&path)?;
    let file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = tokio::io::BufWriter::new(file);

    let mut rows = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, COALESCE(artist, ''), COALESCE(album, ''), title
         FROM songs
         ORDER BY artist, album, disc_number, track, title",
    )
    .fetch(pool);
    let mut chunk = String::with_capacity(64 * 1024);
    while let Some((id, artist, album, title)) = rows.try_next().await? {
        // Writing to a String cannot fail; the heap is the only cost.
        let _ = writeln!(chunk, "{artist}\t{album}\t{title}\t{id}");
        if chunk.len() >= 64 * 1024 {
            writer
                .write_all(chunk.as_bytes())
                .await
                .with_context(|| format!("failed to write {}", path.display()))?;
            chunk.clear();
        }
    }
    if !chunk.is_empty() {
        writer
            .write_all(chunk.as_bytes())
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    writer
        .flush()
        .await
        .with_context(|| format!("failed to flush {}", path.display()))?;
    Ok(())
}

pub(super) async fn upsert_meta<'e, E>(executor: E, key: &str, value: &str) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(executor)
    .await?;
    Ok(())
}
