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

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use sqlx::AssertSqlSafe;
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
/// bookkeeping values (legacy fixture; production uses [`apply`]).
///
/// Returns the song/album/artist/cover ids that changed so the caller can
/// refresh the derived caches (covers, lyrics, descriptions) for exactly
/// those rows.
#[cfg(test)]
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
#[cfg(test)]
pub async fn last_modified(pool: &SqlitePool) -> Result<i64> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'last_modified'")
            .fetch_optional(pool)
            .await?;
    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

/// Snapshot shown by the right status bar.
///
/// N-16: the sync calls this after every pass, including shallow no-ops. The
/// library revision recorded by [`apply`] lets an unchanged library skip the
/// full-table read and the file rewrite; the file must still exist, so a
/// deleted mirror is recreated on the next sync.
pub async fn write_mirror(pool: &SqlitePool) -> Result<()> {
    use std::fmt::Write as _;
    use tokio::io::AsyncWriteExt as _;

    let path = crate::db::mirror_path()?;
    let revision = library_revision(pool).await?;
    if let Some(revision) = revision
        && meta_revision(pool, "mirror_rev").await? == Some(revision)
        && tokio::fs::try_exists(&path).await.unwrap_or(false)
    {
        return Ok(());
    }
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
        _ = writeln!(chunk, "{artist}\t{album}\t{title}\t{id}");
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
    if let Some(revision) = revision {
        upsert_meta(pool, "mirror_rev", &revision.to_string()).await?;
    }
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

/// Reads one integer-valued `meta` row; malformed values count as absent.
async fn meta_revision(pool: &SqlitePool, key: &str) -> Result<Option<i64>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value.and_then(|value| value.trim().parse().ok()))
}

/// Current library revision (`meta.library_rev`): a monotonic counter bumped
/// by every sync pass that actually wrote the catalog. The derived caches (the
/// FTS table, the `index.txt` mirror, the in-memory search catalog) record the
/// revision they were built from and skip an unchanged rebuild (N-16).
pub async fn library_revision(pool: &SqlitePool) -> Result<Option<i64>> {
    meta_revision(pool, "library_rev").await
}

/// Seeds [`library_revision`] on a pre-revision database. Without this, a
/// no-op sync would leave the revision unknown (`None`) forever and the
/// derived caches could never start short-circuiting on an upgraded library.
async fn ensure_library_revision(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<()> {
    let current: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'library_rev'")
            .fetch_optional(&mut **tx)
            .await?;
    if current.is_none() {
        upsert_meta(&mut **tx, "library_rev", "1").await?;
    }
    Ok(())
}

/// Bumps [`library_revision`] inside the sync transaction.
async fn bump_library_revision(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<()> {
    let current: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'library_rev'")
            .fetch_optional(&mut **tx)
            .await?;
    let next = current
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    upsert_meta(&mut **tx, "library_rev", &next.to_string()).await
}

/// A compact pre-fetch view of one album row, used to decide whether its
/// songs need re-fetching. A song move/change alters the album's song count,
/// duration or cover, so a hash change is the song-fetch trigger.
#[derive(Debug, Clone, Copy)]
pub struct AlbumManifest {
    pub hash: u64,
}

/// Server-side album identity hash (same shape as the stored snapshot).
pub fn album_hash(album: &AlbumID3) -> u64 {
    snapshot_hash(&AlbumSnapshot::from_album(album))
}

/// Server-side artist identity hash.
pub fn artist_hash(artist: &ArtistID3) -> u64 {
    snapshot_hash(&ArtistSnapshot::from_artist(artist))
}

/// Server-side song identity hash.
pub fn song_hash(song: &Child) -> u64 {
    snapshot_hash(&SongSnapshot::from_child(song))
}

/// Loads the stored album manifest (id -> hash) before fetching.
pub async fn album_manifest(pool: &SqlitePool) -> Result<HashMap<String, AlbumManifest>> {
    let mut out = HashMap::new();
    let mut rows = sqlx::query_as::<_, AlbumSnapshot>(
        "SELECT id, artist_id, name, artist, year, genre, song_count, duration, cover_art,
                created
         FROM albums",
    )
    .fetch(pool);
    while let Some(album) = rows.try_next().await? {
        out.insert(
            album.id.clone(),
            AlbumManifest {
                hash: snapshot_hash(&album),
            },
        );
    }
    Ok(out)
}

/// Library row counts as `(songs, albums, artists)` for the sync report.
pub async fn counts(pool: &SqlitePool) -> Result<(i64, i64, i64)> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM songs),
                (SELECT COUNT(*) FROM albums),
                (SELECT COUNT(*) FROM artists)",
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// When the last deep (all-album-songs) sync happened. 0 = never.
pub async fn full_sync_at(pool: &SqlitePool) -> i64 {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM meta WHERE key = 'full_sync_at'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    value.and_then(|value| value.parse().ok()).unwrap_or(0)
}

/// A stale/absent getIndexes timestamp cannot be trusted to notice song moves,
/// so a full song pass is forced periodically even when the shallow manifest
/// looks unchanged.
pub async fn full_sync_due(pool: &SqlitePool) -> bool {
    let last = full_sync_at(pool).await;
    last <= 0 || crate::consts::now_secs() - last >= crate::consts::FULL_SYNC_SECS
}

/// Reconciles the library with a full manifest and, for the albums that need
/// it, their songs. Unlike [`replace`], this never deletes rows it did not
/// fetch: unchanged albums keep their songs, while new/changed/moved/deleted
/// albums are written as a delta in one transaction.
///
/// `deep` also prunes artists/albums/songs that the manifest no longer lists.
/// A shallow pass only removes songs inside albums it actually re-fetched, so
/// a stale/partial upstream index can never delete the rest of the library.
/// The full manifest must already be checked by the caller; empty IDs are
/// filtered here for servers that emit malformed rows.
///
/// N-16: this is a real fast path. It hashes only the songs of the albums it
/// re-fetched (a shallow no-op fetches none, so the full `songs` scan is
/// skipped), stages only the temp tables the deletes below use, rewrites only
/// rows that actually changed, and bumps [`library_revision`] only when the
/// catalog changed — which is what lets the post-sync FTS/catalog rebuilds and
/// the mirror rewrite short-circuit.
pub async fn apply(
    pool: &SqlitePool,
    artists: &[ArtistID3],
    albums: &[AlbumID3],
    fetched_songs: &HashMap<String, Vec<Child>>,
    last_modified: i64,
    deep: bool,
) -> Result<SyncChanges> {
    let artists: Vec<&ArtistID3> = artists.iter().filter(|a| !a.id.is_empty()).collect();
    let albums: Vec<&AlbumID3> = albums.iter().filter(|a| !a.id.is_empty()).collect();
    // N-16: membership used to be an O(albums) `Vec::contains` for every
    // fetched song and every fetched-songs key; a hash set keeps the whole
    // pass linear in its input sizes.
    let album_ids: HashSet<&str> = albums.iter().map(|album| album.id.as_str()).collect();

    let mut tx = pool.begin().await?;
    let mut removed = 0usize;

    // Old compact manifest for change detection.
    let mut old_albums: HashMap<String, (u64, Option<String>)> = HashMap::new();
    {
        let mut rows = sqlx::query_as::<_, AlbumSnapshot>(
            "SELECT id, artist_id, name, artist, year, genre, song_count, duration, cover_art,
                    created
             FROM albums",
        )
        .fetch(&mut *tx);
        while let Some(album) = rows.try_next().await? {
            old_albums.insert(album.id.clone(), (snapshot_hash(&album), album.cover_art));
        }
    }
    // The snapshot hash deliberately omits server-owned fields that a sync
    // still rewrites (`starred`/`rating` on albums, `starred` on artists), so
    // the no-op fast path below compares them separately.
    let mut old_album_flags: HashMap<String, (Option<String>, i64)> = HashMap::new();
    {
        let rows: Vec<(String, Option<String>, i64)> =
            sqlx::query_as("SELECT id, starred, rating FROM albums")
                .fetch_all(&mut *tx)
                .await?;
        old_album_flags.extend(
            rows.into_iter()
                .map(|(id, starred, rating)| (id, (starred, rating))),
        );
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
    let mut old_artist_starred: HashMap<String, Option<String>> = HashMap::new();
    {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as("SELECT id, starred FROM artists")
            .fetch_all(&mut *tx)
            .await?;
        old_artist_starred.extend(rows);
    }

    // N-16: diff only the songs of the albums this pass re-fetched. A shallow
    // sync that fetched no album cannot touch the songs table at all, so its
    // full-table scan is skipped entirely.
    let fetched_album_ids: Vec<&str> = fetched_songs
        .keys()
        .filter(|album_id| album_ids.contains(album_id.as_str()))
        .map(String::as_str)
        .collect();
    let mut old_songs: HashMap<String, u64> = HashMap::new();
    for chunk in fetched_album_ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT id, album_id, artist_id, title, album, artist, track, disc_number, year,
                    genre, duration, bit_rate, suffix, size, path, cover_art, created
             FROM songs WHERE album_id IN (",
        );
        crate::cache::push_in_list(&mut builder, chunk);
        let mut rows = builder.build_query_as::<SongSnapshot>().fetch(&mut *tx);
        while let Some(song) = rows.try_next().await? {
            old_songs.insert(song.id.clone(), snapshot_hash(&song));
        }
    }

    let mut changes = SyncChanges::default();
    let mut write_songs: Vec<&Child> = Vec::new();
    // One song can be listed under two albums (N-06). The id is what matters
    // for the diff/keep-set, so keep the first listing and skip the repeat.
    let mut written_ids: HashSet<&str> = HashSet::with_capacity(fetched_songs.len());
    for (album_id, songs) in fetched_songs {
        if !album_ids.contains(album_id.as_str()) {
            continue;
        }
        for song in songs {
            if song.id.is_empty() || !written_ids.insert(song.id.as_str()) {
                continue;
            }
            write_songs.push(song);
            let changed = old_songs
                .get(&song.id)
                .is_none_or(|old| *old != song_hash(song));
            if changed {
                changes.songs.push(song.id.clone());
            }
        }
    }
    let mut write_albums: Vec<&AlbumID3> = Vec::new();
    for album in &albums {
        let seen = old_albums.get(&album.id);
        if seen.is_none_or(|(hash, _)| *hash != album_hash(album)) {
            write_albums.push(*album);
            changes.albums.push(album.id.clone());
            let cover_changed = seen.is_none_or(|(_, cover)| cover != &album.cover_art);
            if cover_changed
                && let Some(cover) = album.cover_art.clone().filter(|cover| !cover.is_empty())
            {
                changes.covers.push(cover);
            }
            continue;
        }
        // Metadata is unchanged; the row is still rewritten when a server-
        // owned field the hash omits changed (album rating/favourite).
        let rating = i64::from(album.user_rating.unwrap_or(0).clamp(0, 5));
        let flags_changed = old_album_flags
            .get(&album.id)
            .is_none_or(|(starred, old_rating)| starred != &album.starred || *old_rating != rating);
        if flags_changed {
            write_albums.push(*album);
        }
    }
    let mut write_artists: Vec<&ArtistID3> = Vec::new();
    for artist in &artists {
        let metadata_changed = old_artists
            .get(&artist.id)
            .is_none_or(|old| *old != artist_hash(artist));
        if metadata_changed {
            write_artists.push(*artist);
            changes.artists.push(artist.id.clone());
            continue;
        }
        // Same rule as albums: the artist favourite timestamp is not part of
        // the snapshot hash but a sync must still import a server-side star.
        let star_changed = old_artist_starred
            .get(&artist.id)
            .is_none_or(|starred| starred != &artist.starred);
        if star_changed {
            write_artists.push(*artist);
        }
    }
    drop(old_songs);
    drop(old_albums);
    drop(old_album_flags);
    drop(old_artists);
    drop(old_artist_starred);

    // N-16 fast path: a shallow pass that re-fetched no album and found no row
    // to write cannot delete or rewrite anything (shallow deletes live only
    // inside re-fetched albums). Skip the temp-table staging, the catalogue
    // rewrites and the revision bump; only the sync bookkeeping advances, so
    // the post-sync derived-cache rebuilds can short-circuit too.
    if !deep && fetched_album_ids.is_empty() && write_albums.is_empty() && write_artists.is_empty()
    {
        upsert_meta(&mut *tx, "last_modified", &last_modified.to_string()).await?;
        upsert_meta(
            &mut *tx,
            "last_sync",
            &crate::consts::now_secs().to_string(),
        )
        .await?;
        ensure_library_revision(&mut tx).await?;
        tx.commit().await?;
        return Ok(changes);
    }

    // Stage the id sets the deletes below need in TEMP tables; the deletes are
    // set-difference based, so a partial fetch can never erase the catalog.
    // `_sync_new_artists`/`_sync_new_albums` only drive the deep prune, so a
    // shallow pass does not pay for them.
    let mut staged: Vec<&'static str> = vec!["_sync_fetched_albums", "_sync_new_songs"];
    if deep {
        staged.extend(["_sync_new_artists", "_sync_new_albums"]);
    }
    for table in staged {
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TEMP TABLE IF NOT EXISTS {table} (id TEXT PRIMARY KEY)"
        )))
        .execute(&mut *tx)
        .await?;
        sqlx::query(AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(&mut *tx)
            .await?;
    }
    if deep {
        crate::cache::insert_many(
            &mut tx,
            "INSERT OR IGNORE INTO _sync_new_artists (id) ",
            &artists,
            |mut row, artist| {
                row.push_bind(&artist.id);
            },
        )
        .await?;
        crate::cache::insert_many(
            &mut tx,
            "INSERT OR IGNORE INTO _sync_new_albums (id) ",
            &albums,
            |mut row, album| {
                row.push_bind(&album.id);
            },
        )
        .await?;
    }
    crate::cache::insert_many(
        &mut tx,
        "INSERT OR IGNORE INTO _sync_fetched_albums (id) ",
        &fetched_album_ids,
        |mut row, album_id| {
            row.push_bind(*album_id);
        },
    )
    .await?;
    // N-06: one song can be listed under two albums; the id is the temp
    // table's primary key, so a plain INSERT aborted the whole sync with a
    // UNIQUE violation. The row is only a keep-set marker, so ignore repeats.
    crate::cache::insert_many(
        &mut tx,
        "INSERT OR IGNORE INTO _sync_new_songs (id) ",
        &write_songs,
        |mut row, song| {
            row.push_bind(&song.id);
        },
    )
    .await?;

    // Artists/albums: replace the authoritative metadata of the rows that
    // changed, then drop the vanished ones on a deep pass.
    crate::cache::insert_many(
        &mut tx,
        "INSERT OR REPLACE INTO artists (id, name, album_count, cover_art, image_url, starred) ",
        &write_artists,
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
    if deep {
        removed +=
            sqlx::query("DELETE FROM artists WHERE id NOT IN (SELECT id FROM _sync_new_artists)")
                .execute(&mut *tx)
                .await?
                .rows_affected() as usize;
    }

    crate::cache::insert_many(
        &mut tx,
        "INSERT OR REPLACE INTO albums
             (id, artist_id, name, artist, year, genre, song_count, duration, cover_art, starred,
              rating, created) ",
        &write_albums,
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
    if deep {
        removed +=
            sqlx::query("DELETE FROM albums WHERE id NOT IN (SELECT id FROM _sync_new_albums)")
                .execute(&mut *tx)
                .await?
                .rows_affected() as usize;
    }

    // Songs: upsert fetched rows, then drop rows that the fetched album no
    // longer contains; songs of untouched albums stay as-is.
    crate::cache::insert_many(
        &mut tx,
        "INSERT OR REPLACE INTO songs
             (id, album_id, artist_id, title, album, artist, track, disc_number, year,
              genre, duration, bit_rate, suffix, size, path, cover_art, play_count, starred,
              created) ",
        &write_songs,
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
    removed += sqlx::query(
        "DELETE FROM songs
         WHERE album_id IN (SELECT id FROM _sync_fetched_albums)
           AND id NOT IN (SELECT id FROM _sync_new_songs)",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected() as usize;
    // Removed albums (or malformed/empty album ids) take their songs with them.
    if deep {
        removed += sqlx::query(
            "DELETE FROM songs WHERE album_id IS NOT NULL
               AND album_id NOT IN (SELECT id FROM _sync_new_albums)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    }

    // N-16: only a pass that actually changed rows advances the revision the
    // derived caches (FTS table, in-memory catalog, index.txt mirror) watch.
    if !write_albums.is_empty()
        || !write_artists.is_empty()
        || !write_songs.is_empty()
        || removed > 0
    {
        bump_library_revision(&mut tx).await?;
    }

    upsert_meta(&mut *tx, "last_modified", &last_modified.to_string()).await?;
    upsert_meta(
        &mut *tx,
        "last_sync",
        &crate::consts::now_secs().to_string(),
    )
    .await?;
    if deep {
        upsert_meta(
            &mut *tx,
            "full_sync_at",
            &crate::consts::now_secs().to_string(),
        )
        .await?;
    }

    tx.commit().await?;
    changes.removed = removed;
    Ok(changes)
}
