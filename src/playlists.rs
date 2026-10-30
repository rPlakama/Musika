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

//! ARC-44 M0: the local half of the playlist index — the m3u folder.
//!
//! `config_dir()/playlists/*.m3u` (`MUSIKA_PLAYLISTS_DIR` overrides) is
//! scanned into `playlist.db` with `source = 'm3u'`; the files stay the
//! source of truth, so a scan rewrites their rows wholesale and playlists
//! whose file vanished are purged. Entries resolve against `library.db` by
//! exact path, then basename, then the `#EXTINF` "Artist - Title" — an
//! unresolved entry keeps `song_id` NULL (rendered dimmed, skipped on play).

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::db::Dbs;
use crate::http_client::SubsonicClient;

/// A parsed m3u entry: the raw path plus whatever `#EXTINF` carried.
#[derive(Debug, Clone, PartialEq)]
pub struct M3uEntry {
    pub path: String,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub duration: Option<i64>,
}

/// The folder the app owns for local playlists.
pub fn m3u_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("MUSIKA_PLAYLISTS_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    Ok(crate::db::config_dir()?.join("playlists"))
}

/// The segment refresh: the m3u scan plus the Subsonic list. A server hit
/// that fails is non-fatal — the cached rows and the m3u half still serve.
pub async fn refresh(dbs: &Dbs, client: &SubsonicClient) -> Result<String> {
    scan_dir(dbs, &m3u_dir()?).await?;
    if let Err(err) = refresh_subsonic(dbs, client).await {
        eprintln!("playlists: could not refresh from the server: {err:#}");
    }
    list_json(dbs).await
}

/// Refreshes the Subsonic half from `getPlaylists`: upserts the rows and
/// purges the playlists that left the server. Contents stay lazy — the page
/// fetches `getPlaylist` when it opens (M2). Returns the list count.
pub async fn refresh_subsonic(dbs: &Dbs, client: &SubsonicClient) -> Result<usize> {
    let lists = client
        .get_playlists()
        .await
        .context("fetching the playlist list")?;
    let now = crate::consts::now_secs();

    let mut tx = dbs.playlist.begin().await?;
    let mut seen: Vec<String> = Vec::with_capacity(lists.len());
    for list in &lists {
        seen.push(list.id.clone());
        sqlx::query(
            "INSERT INTO playlist
                 (id, source, name, comment, owner, public, song_count, duration,
                  created, changed, cover_art, fetched_at)
             VALUES (?1, 'subsonic', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 comment = excluded.comment,
                 owner = excluded.owner,
                 public = excluded.public,
                 song_count = excluded.song_count,
                 duration = excluded.duration,
                 created = excluded.created,
                 changed = excluded.changed,
                 cover_art = excluded.cover_art,
                 fetched_at = excluded.fetched_at",
        )
        .bind(&list.id)
        .bind(&list.name)
        .bind(&list.comment)
        .bind(&list.owner)
        .bind(list.public)
        .bind(list.song_count.unwrap_or(0))
        .bind(list.duration.unwrap_or(0))
        .bind(&list.created)
        .bind(&list.changed)
        .bind(&list.cover_art)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // Purge server playlists that vanished — never inside the library
    // replace transaction (ARC-25 rule): this runs on its own pool.
    let known: Vec<(String,)> = sqlx::query_as("SELECT id FROM playlist WHERE source = 'subsonic'")
        .fetch_all(&mut *tx)
        .await?;
    for (id,) in known {
        if !seen.iter().any(|s| s == &id) {
            sqlx::query("DELETE FROM playlist WHERE id = ?1")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await.context("storing the server playlists")?;
    Ok(seen.len())
}

/// Parses m3u/m3u8 text. Tolerant by design: a BOM, CRLF, comments, blanks
/// and unknown `#` directives are skipped; an `#EXTINF:<secs>,<display>`
/// line feeds the entry on the next path line, and "Artist - Title" splits
/// on the first " - ".
pub fn parse_m3u(text: &str) -> Vec<M3uEntry> {
    let mut out = Vec::new();
    let mut pending: Option<(Option<i64>, Option<String>)> = None;
    for raw in text.lines() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (secs, display) = rest.split_once(',').unwrap_or((rest, ""));
            pending = Some((
                secs.trim().parse::<i64>().ok(),
                Some(display.trim().to_string()),
            ));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (duration, display) = pending.take().unwrap_or((None, None));
        let (artist, title) = match display {
            Some(display) if !display.is_empty() => match display.split_once(" - ") {
                Some((artist, title)) => (
                    Some(artist.trim().to_string()),
                    Some(title.trim().to_string()),
                ),
                None => (None, Some(display)),
            },
            _ => (None, None),
        };
        out.push(M3uEntry {
            path: line.to_string(),
            artist,
            title,
            duration,
        });
    }
    out
}

/// Matching normalization: backslashes to slashes, a leading `./` dropped,
/// case folded (m3u authors and the server disagree on casing).
fn norm(s: &str) -> String {
    let s = s.replace('\\', "/");
    let s = s.strip_prefix("./").unwrap_or(&s).to_string();
    s.to_lowercase()
}

fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// One library song, as matching needs it.
#[derive(Debug, Clone)]
pub struct SongRef {
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<i64>,
    pub cover_art: Option<String>,
    pub path: Option<String>,
}

/// The resolver: exact path → basename → "artist\ttitle" (all normalized,
/// first hit wins per bucket).
#[derive(Debug, Default)]
pub struct Matcher {
    by_path: HashMap<String, usize>,
    by_base: HashMap<String, usize>,
    by_artist_title: HashMap<String, usize>,
}

impl Matcher {
    pub fn new(songs: &[SongRef]) -> Self {
        let mut matcher = Matcher::default();
        for (i, song) in songs.iter().enumerate() {
            if let Some(path) = song.path.as_deref() {
                matcher.by_path.entry(norm(path)).or_insert(i);
                matcher.by_base.entry(norm(basename(path))).or_insert(i);
            }
            if let Some(artist) = song.artist.as_deref() {
                matcher
                    .by_artist_title
                    .entry(format!("{}\t{}", norm(artist), norm(&song.title)))
                    .or_insert(i);
            }
        }
        matcher
    }

    /// Resolves an entry; a relative path is joined against `dir` (the m3u's
    /// own directory).
    pub fn resolve(&self, entry: &M3uEntry, dir: &Path) -> Option<usize> {
        let path = if entry.path.starts_with('/') {
            entry.path.clone()
        } else {
            dir.join(&entry.path).to_string_lossy().to_string()
        };
        if let Some(i) = self.by_path.get(&norm(&path)) {
            return Some(*i);
        }
        if let Some(i) = self.by_base.get(&norm(basename(&path))) {
            return Some(*i);
        }
        if let (Some(artist), Some(title)) = (&entry.artist, &entry.title) {
            if let Some(i) = self
                .by_artist_title
                .get(&format!("{}\t{}", norm(artist), norm(title)))
            {
                return Some(*i);
            }
        }
        None
    }
}

async fn load_songs(pool: &SqlitePool) -> Result<Vec<SongRef>> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
        ),
    >("SELECT id, title, artist, album, duration, cover_art, path FROM songs")
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, title, artist, album, duration, cover_art, path)| SongRef {
                id,
                title,
                artist,
                album,
                duration,
                cover_art,
                path,
            },
        )
        .collect())
}

/// Scans the default m3u folder into `playlist.db` and returns the segment
/// payload (the list rows as JSON) so one call serves the UI.
pub async fn scan(dbs: &Dbs) -> Result<String> {
    scan_dir(dbs, &m3u_dir()?).await
}

/// [`scan`] against an explicit folder (the test seam).
pub async fn scan_dir(dbs: &Dbs, dir: &Path) -> Result<String> {
    std::fs::create_dir_all(dir).ok();

    struct ScannedM3u {
        id: String,
        name: String,
        path: PathBuf,
        mtime: i64,
        entries: Vec<M3uEntry>,
    }

    let mut files: Vec<ScannedM3u> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "m3u" && ext != "m3u8" {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let id = format!("m3u:{name}");
            // Marked seen before the read: an unreadable file keeps its
            // previous rows instead of being purged as vanished.
            seen.push(id.clone());
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            files.push(ScannedM3u {
                id,
                name,
                path,
                mtime,
                entries: parse_m3u(&text),
            });
        }
    }

    // MEMORY: only read the library when a playlist actually has entries to
    // resolve. Opening the Playlists segment used to load every song and
    // build three normalized maps even with an empty m3u folder (FIX-28).
    let needs_library = files.iter().any(|file| !file.entries.is_empty());
    let songs = if needs_library {
        load_songs(&dbs.library).await?
    } else {
        Vec::new()
    };
    let matcher = Matcher::new(&songs);
    for file in &files {
        store_file(
            dbs,
            &file.id,
            &file.name,
            &file.path,
            file.mtime,
            &file.entries,
            &songs,
            &matcher,
        )
        .await?;
    }

    // Purge m3u playlists whose file vanished.
    let known: Vec<(String,)> = sqlx::query_as("SELECT id FROM playlist WHERE source = 'm3u'")
        .fetch_all(&dbs.playlist)
        .await?;
    for (id,) in known {
        if !seen.iter().any(|s| s == &id) {
            sqlx::query("DELETE FROM playlist WHERE id = ?1")
                .bind(&id)
                .execute(&dbs.playlist)
                .await?;
        }
    }

    list_json(dbs).await
}

#[allow(clippy::too_many_arguments)]
async fn store_file(
    dbs: &Dbs,
    id: &str,
    name: &str,
    file: &Path,
    mtime: i64,
    entries: &[M3uEntry],
    songs: &[SongRef],
    matcher: &Matcher,
) -> Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    let now = crate::consts::now_secs();

    let mut duration_total: i64 = 0;
    let mut resolved: Vec<(i64, Option<&SongRef>, &M3uEntry)> = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let song = matcher.resolve(entry, dir).map(|i| &songs[i]);
        duration_total += song
            .and_then(|s| s.duration)
            .or(entry.duration)
            .unwrap_or(0);
        resolved.push((position as i64, song, entry));
    }

    let mut tx = dbs.playlist.begin().await?;
    sqlx::query(
        "INSERT INTO playlist (id, source, name, song_count, duration, path, mtime, fetched_at)
         VALUES (?1, 'm3u', ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             song_count = excluded.song_count,
             duration = excluded.duration,
             path = excluded.path,
             mtime = excluded.mtime,
             fetched_at = excluded.fetched_at",
    )
    .bind(id)
    .bind(name)
    .bind(entries.len() as i64)
    .bind(duration_total)
    .bind(file.to_string_lossy().to_string())
    .bind(mtime)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM playlist_song WHERE playlist_id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    for (position, song, entry) in resolved {
        let (song_id, title, artist, album, duration, cover_art) = match song {
            Some(song) => (
                Some(song.id.as_str()),
                song.title.clone(),
                song.artist.clone(),
                song.album.clone(),
                song.duration,
                song.cover_art.clone(),
            ),
            None => (
                None,
                entry
                    .title
                    .clone()
                    .unwrap_or_else(|| basename(&entry.path).to_string()),
                entry.artist.clone(),
                None,
                entry.duration,
                None,
            ),
        };
        sqlx::query(
            "INSERT INTO playlist_song
                 (playlist_id, position, song_id, title, artist, album, duration, cover_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(id)
        .bind(position)
        .bind(song_id)
        .bind(&title)
        .bind(&artist)
        .bind(&album)
        .bind(duration)
        .bind(&cover_art)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await.context("storing the m3u playlist")?;
    Ok(())
}

/// One segment row. Serializes straight from the query instead of building a
/// `Value` map per playlist.
#[derive(Debug, sqlx::FromRow, serde::Serialize)]
struct PlaylistListRow {
    id: String,
    source: String,
    name: String,
    song_count: i64,
    duration: i64,
    owner: Option<String>,
}

/// The segment payload: every playlist, m3u and (later) Subsonic, as the
/// list rows the QML maps.
pub async fn list_json(dbs: &Dbs) -> Result<String> {
    let rows: Vec<PlaylistListRow> = sqlx::query_as(
        "SELECT id, source, name, song_count, duration, owner
         FROM playlist
         WHERE song_count > 0
         ORDER BY lower(name)",
    )
    .fetch_all(&dbs.playlist)
    .await?;
    let rows: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            let name = row.name;
            let owner = row.owner;
            let origin = if row.source == "m3u" {
                "M3U".to_string()
            } else if let Some(owner) = owner.as_ref().filter(|owner| !owner.is_empty()) {
                owner.clone()
            } else {
                "Playlist".to_string()
            };
            serde_json::json!({
                "id": id.clone(),
                "source": row.source,
                "name": name.clone(),
                "song_count": row.song_count,
                "duration": row.duration,
                "owner": owner,
                "title": name,
                "subtitle": format!("{origin} · {} song{}", row.song_count,
                    if row.song_count == 1 { "" } else { "s" }),
                "meta": if row.duration > 0 {
                    crate::browse::rows::format_duration(Some(row.duration as i32))
                } else {
                    String::new()
                },
                "playlistId": id,
                "songId": "",
                "albumId": "",
                "cover": "",
                "starred": false,
                "rating": 0,
            })
        })
        .collect();
    Ok(serde_json::to_string(&rows)?)
}

/// The playlist page payload: identity, song rows and the first three distinct
/// song covers for the header collage. Rows keep playlist order and duplicates.
pub async fn page_json(dbs: &Dbs, playlist_id: &str, offline: bool) -> Result<String> {
    #[derive(sqlx::FromRow)]
    struct PlaylistRow {
        id: String,
        source: String,
        name: String,
        comment: Option<String>,
        owner: Option<String>,
        song_count: i64,
        duration: i64,
    }
    #[derive(sqlx::FromRow)]
    struct SongRow {
        position: i64,
        song_id: Option<String>,
        title: String,
        artist: Option<String>,
        album: Option<String>,
        duration: Option<i64>,
        cover_art: Option<String>,
    }

    let playlist = sqlx::query_as::<_, PlaylistRow>(
        "SELECT id, source, name, comment, owner, song_count, duration
         FROM playlist WHERE id = ?1",
    )
    .bind(playlist_id)
    .fetch_optional(&dbs.playlist)
    .await?
    .ok_or_else(|| anyhow::anyhow!("playlist not found"))?;

    let rows = sqlx::query_as::<_, SongRow>(
        "SELECT position, song_id, title, artist, album, duration, cover_art
         FROM playlist_song WHERE playlist_id = ?1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&dbs.playlist)
    .await?;

    let ids: Vec<String> = rows.iter().filter_map(|row| row.song_id.clone()).collect();
    let library = crate::library::songs_by_ids(&dbs.library, &ids).await?;
    let by_id: HashMap<String, crate::library::SongRow> =
        library.into_iter().map(|song| (song.id.clone(), song)).collect();
    let cover_ids: Vec<Option<String>> = rows
        .iter()
        .map(|row| row.cover_art.clone())
        .chain(by_id.values().map(|song| song.cover_art.clone()))
        .collect();
    let covers = crate::covers::url_map_for(&dbs, cover_ids.iter()).await?;
    let filter = crate::offline::load(dbs, offline).await?;

    let mut collage: Vec<String> = Vec::new();
    let mut songs = Vec::with_capacity(rows.len());
    for row in &rows {
        let library_song = row
            .song_id
            .as_ref()
            .and_then(|id| by_id.get(id));
        let cover = row
            .cover_art
            .as_ref()
            .and_then(|id| covers.get(id).cloned())
            .or_else(|| {
                library_song
                    .and_then(|song| song.cover_art.as_ref())
                    .and_then(|id| covers.get(id).cloned())
            })
            .unwrap_or_default();
        if !cover.is_empty() && !collage.contains(&cover) && collage.len() < 3 {
            collage.push(cover.clone());
        }
        let playable = library_song.is_some()
            && filter
                .as_ref()
                .is_none_or(|filter| filter.contains_song(row.song_id.as_deref().unwrap_or("")));
        let duration = row
            .duration
            .or_else(|| library_song.and_then(|song| song.duration.map(i64::from)));
        let album_id = library_song
            .and_then(|song| song.album_id.clone())
            .unwrap_or_default();
        let track = library_song
            .and_then(|song| song.track)
            .unwrap_or(row.position as i32 + 1);
        songs.push(serde_json::json!({
            "position": row.position,
            "track": track.to_string(),
            "title": row.title,
            "subtitle": row.artist.clone().or_else(|| library_song.and_then(|s| s.artist.clone())).unwrap_or_default(),
            "album": row.album.clone().or_else(|| library_song.and_then(|s| s.album.clone())).unwrap_or_default(),
            "duration": duration.map(|seconds| crate::browse::rows::format_duration(Some(seconds as i32))).unwrap_or_default(),
            "cover": cover,
            "songId": row.song_id.clone().unwrap_or_default(),
            "albumId": album_id,
            "starred": library_song.and_then(|song| song.starred.as_ref()).is_some(),
            "playable": playable,
        }));
    }

    let origin = if playlist.source == "m3u" {
        "M3U".to_string()
    } else if let Some(owner) = playlist.owner.as_ref().filter(|owner| !owner.is_empty()) {
        owner.clone()
    } else {
        "Playlist".to_string()
    };
    Ok(serde_json::json!({
        "playlist_id": playlist.id,
        "name": playlist.name,
        "comment": playlist.comment.unwrap_or_default(),
        "subtitle": format!("{origin} · {} song{}", playlist.song_count,
            if playlist.song_count == 1 { "" } else { "s" }),
        "meta": if playlist.duration > 0 {
            crate::browse::rows::format_duration(Some(playlist.duration as i32))
        } else {
            String::new()
        },
        "song_count": playlist.song_count,
        "collage": collage,
        "songs": songs,
    })
    .to_string())
}
