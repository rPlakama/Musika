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
    let known: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM playlist WHERE source = 'subsonic'")
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
            pending = Some((secs.trim().parse::<i64>().ok(), Some(display.trim().to_string())));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (duration, display) = pending.take().unwrap_or((None, None));
        let (artist, title) = match display {
            Some(display) if !display.is_empty() => match display.split_once(" - ") {
                Some((artist, title)) => {
                    (Some(artist.trim().to_string()), Some(title.trim().to_string()))
                }
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
                matcher
                    .by_base
                    .entry(norm(basename(path)))
                    .or_insert(i);
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
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<i64>, Option<String>, Option<String>)>(
        "SELECT id, title, artist, album, duration, cover_art, path FROM songs",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, artist, album, duration, cover_art, path)| SongRef {
            id,
            title,
            artist,
            album,
            duration,
            cover_art,
            path,
        })
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
    let known: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM playlist WHERE source = 'm3u'")
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
                entry.title.clone().unwrap_or_else(|| {
                    basename(&entry.path).to_string()
                }),
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
    Ok(serde_json::to_string(&rows)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_pool() -> SqlitePool {
        crate::sqlite::memory_pool(None).await
    }

    fn test_dbs(pools: &SqlitePool, playlist: &SqlitePool) -> Dbs {
        Dbs {
            credentials: pools.clone(),
            library: pools.clone(),
            search: pools.clone(),
            covers: pools.clone(),
            media: pools.clone(),
            lyrics: pools.clone(),
            colors: pools.clone(),
            status: pools.clone(),
            artist_user: pools.clone(),
            download: pools.clone(),
            playlist: playlist.clone(),
        }
    }

    #[tokio::test]
    async fn list_skips_empty_playlists() {
        let library = memory_pool().await;
        let playlist = memory_pool().await;
        sqlx::migrate!("./migrations_playlist")
            .run(&playlist)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO playlist (id, source, name, song_count, fetched_at) VALUES
             ('p0', 'subsonic', 'Empty', 0, 0),
             ('p1', 'subsonic', 'Mix', 2, 0)",
        )
        .execute(&playlist)
        .await
        .unwrap();

        let json = list_json(&test_dbs(&library, &playlist)).await.unwrap();
        let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "p1");
    }

    #[tokio::test]
    async fn scan_indexes_m3u_files() {
        let library = memory_pool().await;
        sqlx::migrate!("./migrations_library")
            .run(&library)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO songs (id, title, artist, album, duration, path) VALUES
             ('s1', 'ATW', 'All Them Witches', 'ATW', 500, 'Music/a/atw.flac'),
             ('s2', 'Blue Lines', 'Massive Attack', NULL, 300, 'Music/m/blue.flac')",
        )
        .execute(&library)
        .await
        .unwrap();
        let playlist = memory_pool().await;
        sqlx::migrate!("./migrations_playlist")
            .run(&playlist)
            .await
            .unwrap();
        let dbs = test_dbs(&library, &playlist);

        let dir = std::env::temp_dir().join(format!("musika-pl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mix.m3u"),
            "#EXTM3U\n#EXTINF:500,All Them Witches - ATW\nsomewhere/atw.flac\n\
             #EXTINF:300,Massive Attack - Blue Lines\nother/blue.flac\n\
             #EXTINF:1,Ghost - Missing\nghost.flac\n",
        )
        .unwrap();
        let json = scan_dir(&dbs, &dir).await.unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let lists = parsed.as_array().unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0]["name"], "mix.m3u");
        assert_eq!(lists[0]["song_count"], 3);

        let rows: Vec<(i64, Option<String>)> =
            sqlx::query_as("SELECT position, song_id FROM playlist_song ORDER BY position")
                .fetch_all(&playlist)
                .await
                .unwrap();
        assert_eq!(rows.len(), 3);
        // Basename fallback, then artist-title fallback, then unmatched kept.
        assert_eq!(rows[0].1.as_deref(), Some("s1"));
        assert_eq!(rows[1].1.as_deref(), Some("s2"));
        assert_eq!(rows[2].1, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty m3u folder must not read `library.db` at all: the segment is
    /// often opened on a machine with no local playlists, and the old scan
    /// loaded every song and built three normalized lookup maps first. The
    /// library pool here has no schema, so a read would fail the scan.
    #[tokio::test]
    async fn an_empty_m3u_folder_does_not_read_the_library() {
        let library = memory_pool().await;
        let playlist = memory_pool().await;
        sqlx::migrate!("./migrations_playlist")
            .run(&playlist)
            .await
            .unwrap();
        let dbs = test_dbs(&library, &playlist);

        let dir = std::env::temp_dir().join(format!("musika-pl-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let json = scan_dir(&dbs, &dir).await.expect("empty scan");
        assert_eq!(json, "[]");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_extinf_and_paths() {
        let text = "\u{feff}#EXTM3U\r\n# a comment\r\n#EXTINF:214,All Them Witches - ATW\r\nMusic/atw.flac\r\n\r\n#EXTINF:45,Some Title\r\n./rel/song.mp3\r\nplain.ogg\r\n";
        let entries = parse_m3u(text);
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].path, "Music/atw.flac");
        assert_eq!(entries[0].artist.as_deref(), Some("All Them Witches"));
        assert_eq!(entries[0].title.as_deref(), Some("ATW"));
        assert_eq!(entries[0].duration, Some(214));

        assert_eq!(entries[1].path, "./rel/song.mp3");
        assert_eq!(entries[1].artist, None);
        assert_eq!(entries[1].title.as_deref(), Some("Some Title"));
        assert_eq!(entries[1].duration, Some(45));

        assert_eq!(entries[2].path, "plain.ogg");
        assert_eq!(entries[2].duration, None);
    }

    #[test]
    fn matcher_falls_back_path_then_base_then_names() {
        let songs = vec![
            SongRef {
                id: "s1".into(),
                title: "ATW".into(),
                artist: Some("All Them Witches".into()),
                album: Some("ATW".into()),
                duration: Some(500),
                cover_art: None,
                path: Some("Music/All Them Witches/atw.flac".into()),
            },
            SongRef {
                id: "s2".into(),
                title: "Blue Lines".into(),
                artist: Some("Massive Attack".into()),
                album: None,
                duration: Some(300),
                cover_art: None,
                path: Some("Music/MA/blue.flac".into()),
            },
        ];
        let matcher = Matcher::new(&songs);
        let dir = Path::new("/playlists");

        // Exact server path.
        let entry = M3uEntry {
            path: "Music/All Them Witches/atw.flac".into(),
            artist: None,
            title: None,
            duration: None,
        };
        assert_eq!(matcher.resolve(&entry, dir), Some(0));

        // Basename fallback (different folder).
        let entry = M3uEntry {
            path: "/home/me/somewhere/blue.flac".into(),
            artist: None,
            title: None,
            duration: None,
        };
        assert_eq!(matcher.resolve(&entry, dir), Some(1));

        // Artist - Title fallback (nothing path-like matches).
        let entry = M3uEntry {
            path: "1-02 ATW.flac".into(),
            artist: Some("All Them Witches".into()),
            title: Some("ATW".into()),
            duration: None,
        };
        assert_eq!(matcher.resolve(&entry, dir), Some(0));

        // Unresolved stays unresolved.
        let entry = M3uEntry {
            path: "missing.flac".into(),
            artist: Some("Nobody".into()),
            title: Some("Nothing".into()),
            duration: None,
        };
        assert_eq!(matcher.resolve(&entry, dir), None);
    }
}
