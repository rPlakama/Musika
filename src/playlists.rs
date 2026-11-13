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
//! `<account data root>/playlists/*.m3u` (`MUSIKA_PLAYLISTS_DIR` overrides) is
//! scanned into `playlist.db` with `source = 'm3u'`; the files stay the
//! source of truth, so a scan rewrites their rows wholesale and playlists
//! whose file vanished are purged. Entries resolve against `library.db` by
//! exact path, then basename, then the `#EXTINF` "Artist - Title" — an
//! unresolved entry keeps `song_id` NULL (rendered dimmed, skipped on play).

use anyhow::{Context, Result, anyhow};
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
    if let Some(dir) = std::env::var_os("MUSIKA_PLAYLISTS_DIR")
        && !dir.is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    Ok(crate::db::data_root()?.join("playlists"))
}

/// Validates the name/ids shared by both creation halves: the name is trimmed,
/// blank ids are dropped and at least one real song must remain. Playlists
/// never hold duplicates, so only the newest occurrence of each id survives.
fn checked_playlist<'a>(name: &'a str, song_ids: &[String]) -> Result<(&'a str, Vec<String>)> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("a playlist needs a name");
    }
    let ids: Vec<String> = song_ids
        .iter()
        .filter(|id| !id.trim().is_empty())
        .cloned()
        .collect();
    let ids = dedup_keep_last(&ids);
    if ids.is_empty() {
        anyhow::bail!("a playlist needs at least one song");
    }
    Ok((name, ids))
}

/// Playlist entries are unique. Adding an existing song moves it, so the
/// newest occurrence wins and every older copy is dropped.
fn dedup_keep_last(ids: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids.iter().rev() {
        if seen.insert(id.as_str()) {
            out.push(id.clone());
        }
    }
    out.reverse();
    out
}

/// Insert (or move) ids in an ordered playlist: existing occurrences are
/// removed first, then the newest copies go in at `position` (clamped), so a
/// duplicate add behaves like a position update.
fn upsert_ids(ordered: &mut Vec<String>, position: usize, song_ids: &[String]) {
    let incoming = dedup_keep_last(song_ids);
    if incoming.is_empty() {
        return;
    }
    let wanted: std::collections::HashSet<&str> = incoming.iter().map(String::as_str).collect();
    ordered.retain(|id| !wanted.contains(id.as_str()));
    let at = position.min(ordered.len());
    for (offset, id) in incoming.into_iter().enumerate() {
        ordered.insert((at + offset).min(ordered.len()), id);
    }
}

/// The cleanup rule for duplicates that already exist: keep the first
/// occurrence of every key, drop the later ones and preserve the order of
/// everything that survives. Items without a key are always kept because they
/// cannot be compared.
fn dedup_keep_first<T>(items: Vec<T>, key: impl Fn(&T) -> Option<String>) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match key(&item) {
            Some(key) => {
                if !seen.insert(key) {
                    continue;
                }
                out.push(item);
            }
            None => out.push(item),
        }
    }
    out
}

/// Blank ids are dropped and the first occurrence of each id survives: the
/// duplicate-free shape every playlist view and write path works from.
pub(crate) fn dedup_keep_first_ids(ids: &[String]) -> Vec<String> {
    let ids: Vec<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    dedup_keep_first(ids, |id| Some(id.clone()))
}

/// One local library row as the M3U writer needs it.
struct M3uSong {
    title: String,
    artist: Option<String>,
    duration: Option<i64>,
    path: Option<String>,
}

/// The `songs` projection for [`m3u_text`] (a named row keeps the query and
/// its tuple out of the function body).
#[derive(sqlx::FromRow)]
struct M3uSongRow {
    id: String,
    title: String,
    artist: Option<String>,
    duration: Option<i64>,
    path: Option<String>,
}

/// The M3U body for `song_ids`, preserving their order and duplicates. Local
/// library paths are used when known (the scan resolves those exactly, by
/// basename or by their `#EXTINF` names); anything else falls back to the
/// app-owned `musika:song:<id>` entry [`Matcher`] resolves by song id. One
/// cheap library query decorates every known id, so server-only picks stay
/// plain path lines without extra calls.
pub async fn m3u_text(dbs: &Dbs, song_ids: &[String]) -> Result<String> {
    let ids: Vec<String> = song_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();

    let mut songs: HashMap<String, M3uSong> = HashMap::new();
    if !ids.is_empty() {
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT id, title, artist, duration, path FROM songs WHERE id IN (",
        );
        {
            let mut separated = builder.separated(", ");
            for id in &ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
        }
        let rows: Vec<M3uSongRow> = builder.build_query_as().fetch_all(&dbs.library).await?;
        for row in rows {
            songs.insert(
                row.id,
                M3uSong {
                    title: row.title,
                    artist: row.artist,
                    duration: row.duration,
                    path: row.path,
                },
            );
        }
    }

    let mut text = String::from("#EXTM3U\n");
    for id in &ids {
        let song = songs.get(id);
        if let Some(song) = song {
            let artist = song.artist.as_deref().unwrap_or_default();
            let display = if artist.is_empty() {
                song.title.clone()
            } else {
                format!("{artist} - {}", song.title)
            };
            text.push_str(&format!(
                "#EXTINF:{},{}\n",
                song.duration.unwrap_or(0),
                m3u_line(&display)
            ));
        }
        let path = song
            .and_then(|song| song.path.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("musika:song:{id}"));
        text.push_str(&m3u_line(&path));
        text.push('\n');
    }
    Ok(text)
}

/// A path/display line with embedded newlines flattened (an m3u line break
/// would turn one entry into two).
fn m3u_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

/// A list-friendly file name inside `dir`. The sanitized playlist name is the
/// file identity: saving the same name again overwrites that mix instead of
/// creating a near-duplicate with a timestamp suffix.
fn m3u_filename(_dir: &Path, name: &str) -> String {
    let mut safe = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '(' | ')' | '&' | '\'' | '·') {
            safe.push(ch);
        } else if !safe.ends_with('-') {
            safe.push('-');
        }
    }
    let safe = safe.trim().trim_end_matches('-').trim();
    let safe = if safe.is_empty() { "Playlist" } else { safe };
    format!("{safe}.m3u")
}

/// Creates or replaces a local playlist from song ids: writes the named M3U
/// under [`m3u_dir`] and scans that folder into `playlist.db` so the list is
/// live immediately. An empty (or all-blank) name/id list is rejected.
/// Returns the M3U path that was written.
pub async fn create_local(dbs: &Dbs, name: &str, song_ids: &[String]) -> Result<PathBuf> {
    let (name, ids) = checked_playlist(name, song_ids)?;
    let dir = m3u_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating the playlist folder {}", dir.display()))?;
    let text = m3u_text(dbs, &ids).await?;
    let path = dir.join(m3u_filename(&dir, name));
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    scan_dir(dbs, &dir).await?;
    Ok(path)
}

/// Creates the playlist on the Subsonic server and refreshes the cached list
/// right away. Order is preserved and duplicate ids collapse to their newest
/// occurrence because playlists never hold duplicates. An empty (or all-blank)
/// name/id list is rejected before any network call.
pub async fn create_remote(dbs: &Dbs, name: &str, song_ids: &[String]) -> Result<()> {
    let (name, ids) = checked_playlist(name, song_ids)?;
    let client = crate::session::client(dbs)
        .await
        .context("creating a server playlist needs a signed-in server")?;
    client
        .create_playlist(name, &ids)
        .await
        .context("creating the server playlist")?;
    refresh_subsonic(dbs, &client)
        .await
        .context("refreshing the server playlist list")?;
    Ok(())
}

/// One call for both sources: always writes the app-owned local M3U, then
/// mirrors it to the server when online. Following the saved-radio rule, a
/// failed server copy is logged rather than discarding the local playlist
/// (the returned path is the local file that survived).
pub async fn create(dbs: &Dbs, name: &str, song_ids: &[String], offline: bool) -> Result<PathBuf> {
    let path = create_local(dbs, name, song_ids).await?;
    if !offline && let Err(err) = create_remote(dbs, name, song_ids).await {
        eprintln!("playlists: saved locally, server copy failed: {err:#}");
    }
    Ok(path)
}

/// The segment refresh: the m3u scan plus the Subsonic list. A server hit
/// that fails is non-fatal — the cached rows and the m3u half still serve.
pub async fn refresh(dbs: &Dbs, client: &SubsonicClient) -> Result<Vec<crate::model::Row>> {
    scan_dir(dbs, &m3u_dir()?).await?;
    if let Err(err) = refresh_subsonic(dbs, client).await {
        eprintln!("playlists: could not refresh from the server: {err:#}");
    }
    list(dbs).await
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
    /// App-owned M3U entries written by a saved radio use `musika:song:<id>`.
    by_id: HashMap<String, usize>,
}

impl Matcher {
    pub fn new(songs: &[SongRef]) -> Self {
        let mut matcher = Matcher::default();
        for (i, song) in songs.iter().enumerate() {
            matcher.by_id.entry(song.id.clone()).or_insert(i);
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
        if let Some(id) = entry.path.strip_prefix("musika:song:") {
            return self.by_id.get(id).copied();
        }
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
        if let (Some(artist), Some(title)) = (&entry.artist, &entry.title)
            && let Some(i) = self
                .by_artist_title
                .get(&format!("{}\t{}", norm(artist), norm(title)))
        {
            return Some(*i);
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
/// rows so one call serves the UI.
pub async fn scan(dbs: &Dbs) -> Result<Vec<crate::model::Row>> {
    scan_dir(dbs, &m3u_dir()?).await
}

/// [`scan`] against an explicit folder (the test seam).
pub async fn scan_dir(dbs: &Dbs, dir: &Path) -> Result<Vec<crate::model::Row>> {
    _ = std::fs::create_dir_all(dir);

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

    list(dbs).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the playlist file/database row; a wrapper struct would restate it"
)]
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
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let now = crate::consts::now_secs();

    // The file is the source of truth, so duplicates left by an external edit
    // or an old bug are removed physically here (the first occurrence stays),
    // not just hidden from the stored rows.
    let deduped = dedup_m3u_entries(entries.to_vec(), matcher, songs, dir);
    if deduped.len() != entries.len()
        && let Err(err) = write_m3u_entries(file, &deduped)
    {
        eprintln!(
            "playlists: could not rewrite {} without duplicates: {err:#}",
            file.display()
        );
    }

    let mut duration_total: i64 = 0;
    let mut resolved: Vec<(i64, Option<&SongRef>, &M3uEntry)> = Vec::new();
    for (position, entry) in deduped.iter().enumerate() {
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
    .bind(deduped.len() as i64)
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

/// Where a playlist came from, as the row subtitle leads with it: the local
/// m3u folder, the server owner's name, or a plain playlist.
fn playlist_origin(row: &PlaylistListRow) -> String {
    if row.source == "m3u" {
        "M3U".to_string()
    } else if let Some(owner) = row.owner.as_ref().filter(|owner| !owner.is_empty()) {
        owner.clone()
    } else {
        "Playlist".to_string()
    }
}

/// One playlist hit for the unified search: identity plus the palette text.
#[derive(Debug, Clone)]
pub struct PlaylistHit {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub meta: String,
}

/// One playlist segment row, deriving the same title/subtitle/meta text the
/// JSON row carried (`playlist_row`) without a `Value` round trip.
fn playlist_segment_row(row: &PlaylistListRow) -> crate::model::Row {
    let origin = playlist_origin(row);
    crate::model::Row {
        item: crate::model::ItemMeta {
            id: row.id.clone(),
            title: row.name.clone(),
            cover_url: String::new(),
            cover_remote: String::new(),
            album_id: String::new(),
        },
        track: String::new(),
        kind: crate::model::RowKind::Playlist,
        subtitle: format!(
            "{origin} · {} song{}",
            row.song_count,
            if row.song_count == 1 { "" } else { "s" }
        ),
        meta: if row.duration > 0 {
            crate::browse::rows::format_duration(Some(row.duration as i32))
        } else {
            String::new()
        },
        song_id: String::new(),
        starred: false,
        rating: 0,
    }
}

/// The segment payload: every playlist, m3u and Subsonic, as typed list rows.
pub async fn list(dbs: &Dbs) -> Result<Vec<crate::model::Row>> {
    let rows: Vec<PlaylistListRow> = sqlx::query_as(
        "SELECT id, source, name, song_count, duration, owner
         FROM playlist
         WHERE song_count > 0
         ORDER BY lower(name)",
    )
    .fetch_all(&dbs.playlist)
    .await?;
    Ok(rows.iter().map(playlist_segment_row).collect())
}

/// M2: fetch one Subsonic playlist's contents on demand. The list refresh only
/// stores identities; the page is the first place that needs the ordered
/// entries. Positions are rewritten so server reorders are exact, and the
/// keep-first cleanup removes duplicate entries from both the cache and (best
/// effort) the server copy without re-entering this loader.
async fn load_subsonic_songs(dbs: &Dbs, client: &SubsonicClient, playlist_id: &str) -> Result<()> {
    let page = client.get_playlist(playlist_id).await?;
    let current_order: Vec<String> = page.entry.iter().map(|song| song.id.clone()).collect();
    let deduped = dedup_keep_first(page.entry, |song| {
        let id = song.id.trim();
        (!id.is_empty()).then(|| id.to_string())
    });
    let duplicates = deduped.len() != current_order.len();

    let mut tx = dbs.playlist.begin().await?;
    sqlx::query("DELETE FROM playlist_song WHERE playlist_id = ?1")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await?;
    for (position, song) in deduped.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_song
                 (playlist_id, position, song_id, title, artist, album, duration, cover_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(playlist_id)
        .bind(position as i64)
        .bind(&song.id)
        .bind(&song.title)
        .bind(&song.artist)
        .bind(&song.album)
        .bind(song.duration.map(i64::from))
        .bind(&song.cover_art)
        .execute(&mut *tx)
        .await?;
    }
    // Keep the cached identity in step with the entries actually stored: the
    // page's stale-count check compares the two, and a deduped cache must not
    // keep claiming the server's duplicate count.
    sqlx::query("UPDATE playlist SET song_count = ?2 WHERE id = ?1")
        .bind(playlist_id)
        .bind(deduped.len() as i64)
        .execute(&mut *tx)
        .await?;
    let durations: Vec<i64> = deduped
        .iter()
        .filter_map(|song| song.duration.map(i64::from))
        .collect();
    if !durations.is_empty() {
        let duration_total: i64 = durations.iter().sum();
        sqlx::query("UPDATE playlist SET duration = ?2 WHERE id = ?1")
            .bind(playlist_id)
            .bind(duration_total)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    if duplicates {
        let deduped_order: Vec<String> = deduped.into_iter().map(|song| song.id).collect();
        match clean_server_duplicate_entries(client, playlist_id, &current_order, &deduped_order)
            .await
        {
            Ok(()) => {
                // The list carries the song count/duration the page header and
                // the Playlists list use; only a successful cleanup can make
                // them agree with the deduplicated server copy.
                if let Err(err) = refresh_subsonic(dbs, client).await {
                    eprintln!("playlists: could not refresh after cleaning duplicates: {err:#}");
                }
            }
            Err(err) => {
                eprintln!("playlists: could not clean duplicates from {playlist_id}: {err:#}");
            }
        }
    }
    Ok(())
}

/// Physically remove duplicate server entries: remove every entry in the
/// current order (duplicates included, one removal per occurrence), then
/// re-add the keep-first order. The caller owns the reload/refresh; this never
/// calls [`load_subsonic_songs`], so a server that keeps reporting duplicates
/// cannot loop the cleanup.
async fn clean_server_duplicate_entries(
    client: &SubsonicClient,
    playlist_id: &str,
    current_order: &[String],
    deduped_order: &[String],
) -> Result<()> {
    client
        .update_playlist_remove(playlist_id, current_order)
        .await
        .context("removing the duplicated server playlist entries")?;
    client
        .update_playlist_add(playlist_id, deduped_order)
        .await
        .context("re-adding the deduplicated server playlist order")?;
    Ok(())
}

/// Just the display name, for the startup bootstrap target.
pub async fn name(dbs: &Dbs, playlist_id: &str) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT name FROM playlist WHERE id = ?1")
            .bind(playlist_id)
            .fetch_optional(&dbs.playlist)
            .await?,
    )
}

/// One cached playlist header row for [`page`].
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

/// One cached `playlist_song` row for [`page`].
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

async fn playlist_page_row(dbs: &Dbs, playlist_id: &str) -> Result<PlaylistRow> {
    sqlx::query_as::<_, PlaylistRow>(
        "SELECT id, source, name, comment, owner, song_count, duration
         FROM playlist WHERE id = ?1",
    )
    .bind(playlist_id)
    .fetch_optional(&dbs.playlist)
    .await?
    .ok_or_else(|| anyhow::anyhow!("playlist not found"))
}

async fn playlist_song_rows(dbs: &Dbs, playlist_id: &str) -> Result<Vec<SongRow>> {
    Ok(sqlx::query_as::<_, SongRow>(
        "SELECT position, song_id, title, artist, album, duration, cover_art
         FROM playlist_song WHERE playlist_id = ?1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&dbs.playlist)
    .await?)
}

/// Whether the cached rows list a song id twice: a page holding duplicates
/// must refetch (and, when possible, clean) the server copy even when its
/// count happens to look right.
fn has_duplicate_song_rows(rows: &[SongRow]) -> bool {
    let mut seen = std::collections::HashSet::new();
    rows.iter().any(|row| match row.song_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => !seen.insert(id),
        _ => false,
    })
}

/// The page's keep-first defense: even when the cache (or the server) still
/// holds duplicates, each song is shown once. Unresolved rows carry no id to
/// compare here; the m3u rescan cleans those by normalized path instead.
fn dedup_song_rows(rows: Vec<SongRow>) -> Vec<SongRow> {
    dedup_keep_first(rows, |row| {
        row.song_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    })
}

/// Replace the cached rows with the deduplicated page payload and keep the
/// cached song count/duration in step, so the List view and the page's own
/// stale-count refetch agree with what is shown.
async fn store_deduped_page_rows(
    dbs: &Dbs,
    playlist_id: &str,
    rows: &[SongRow],
    duration: i64,
) -> Result<()> {
    let mut tx = dbs.playlist.begin().await?;
    sqlx::query("DELETE FROM playlist_song WHERE playlist_id = ?1")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await?;
    for (position, row) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO playlist_song
                 (playlist_id, position, song_id, title, artist, album, duration, cover_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(playlist_id)
        .bind(position as i64)
        .bind(&row.song_id)
        .bind(&row.title)
        .bind(&row.artist)
        .bind(&row.album)
        .bind(row.duration)
        .bind(&row.cover_art)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE playlist SET song_count = ?2, duration = ?3 WHERE id = ?1")
        .bind(playlist_id)
        .bind(rows.len() as i64)
        .bind(duration)
        .execute(&mut *tx)
        .await?;
    tx.commit()
        .await
        .context("storing the deduplicated playlist rows")?;
    Ok(())
}

/// The playlist page payload: identity, song rows and every distinct song cover
/// for the header collage. Rows keep playlist order and never show a song
/// twice, even when the cache could not be cleaned.
pub async fn page(
    dbs: &Dbs,
    playlist_id: &str,
) -> Result<(crate::model::PageHeader, Vec<crate::model::Row>)> {
    let mut playlist = playlist_page_row(dbs, playlist_id).await?;
    let mut rows = playlist_song_rows(dbs, playlist_id).await?;

    // The list refresh only updates `song_count`; additions/removals made
    // elsewhere (the server UI, another client) leave `playlist_song` stale.
    // An empty page always fetches and a count mismatch fetches again so the
    // page cannot show fewer songs than its own header claims; a cached row
    // list with duplicates fetches so the server copy can be cleaned too.
    let stale = playlist.source == "subsonic"
        && (rows.is_empty()
            || rows.len() as i64 != playlist.song_count
            || has_duplicate_song_rows(&rows));
    if stale && let Ok(client) = crate::session::client(dbs).await {
        match load_subsonic_songs(dbs, &client, playlist_id).await {
            Ok(()) => {
                // The loader keeps the cached count/duration honest; re-read
                // them so this payload's header matches the reloaded rows.
                let (song_count, duration) = sqlx::query_as::<_, (i64, i64)>(
                    "SELECT song_count, duration FROM playlist WHERE id = ?1",
                )
                .bind(playlist_id)
                .fetch_one(&dbs.playlist)
                .await?;
                playlist.song_count = song_count;
                playlist.duration = duration;
                rows = playlist_song_rows(dbs, playlist_id).await?;
                if rows.len() as i64 != playlist.song_count {
                    // The reloaded entries are the truth for this payload; a
                    // list count that still carries the pre-cleanup number must
                    // not make the header disagree with the rows shown.
                    playlist.song_count = rows.len() as i64;
                    sqlx::query("UPDATE playlist SET song_count = ?2 WHERE id = ?1")
                        .bind(playlist_id)
                        .bind(playlist.song_count)
                        .execute(&dbs.playlist)
                        .await?;
                }
            }
            Err(err) => eprintln!("playlists: could not load {playlist_id}: {err:#}"),
        }
    }

    let cached_rows = rows.len();
    let rows = dedup_song_rows(rows);
    let deduped_locally = rows.len() != cached_rows;

    let ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row.song_id.clone())
        .filter(|id| !id.trim().is_empty())
        .collect();
    let library = crate::library::songs_by_ids(&dbs.library, &ids).await?;
    let by_id: HashMap<String, crate::library::SongRow> = library
        .into_iter()
        .map(|song| (song.id.clone(), song))
        .collect();
    let cover_ids: Vec<Option<String>> = rows
        .iter()
        .map(|row| row.cover_art.clone())
        .chain(by_id.values().map(|song| song.cover_art.clone()))
        .collect();
    let covers = crate::covers::url_map_for(dbs, cover_ids.iter()).await?;
    let remotes = crate::covers::remote_map_for(dbs, cover_ids.iter()).await?;

    let mut duration_total: i64 = 0;
    let mut collage: Vec<String> = Vec::new();
    let mut collage_remote: Vec<String> = Vec::new();
    let mut collage_origins: Vec<String> = Vec::new();
    let mut list_rows: Vec<crate::model::Row> = Vec::with_capacity(rows.len());
    for row in &rows {
        let library_song = row.song_id.as_ref().and_then(|id| by_id.get(id));
        let cover_id = row
            .cover_art
            .as_ref()
            .or_else(|| library_song.and_then(|song| song.cover_art.as_ref()));
        let cover = cover_id
            .and_then(|id| covers.get(id).cloned())
            .unwrap_or_default();
        let cover_remote = cover_id
            .and_then(|id| remotes.get(id).cloned())
            .unwrap_or_default();
        let origin = library_song
            .and_then(|song| song.album_id.clone())
            .filter(|id| !id.is_empty())
            .or_else(|| row.album.clone().filter(|album| !album.is_empty()))
            .unwrap_or_else(|| format!("#{position}", position = row.position));
        if !cover.is_empty() && !collage_origins.contains(&origin) {
            collage_origins.push(origin);
            collage.push(cover.clone());
            collage_remote.push(cover_remote.clone());
        }
        let duration = row
            .duration
            .or_else(|| library_song.and_then(|song| song.duration.map(i64::from)));
        duration_total += duration.unwrap_or(0);
        let album_id = library_song
            .and_then(|song| song.album_id.clone())
            .unwrap_or_default();
        let song_id = row.song_id.clone().unwrap_or_default();
        list_rows.push(crate::model::Row {
            item: crate::model::ItemMeta {
                id: song_id.clone(),
                title: row.title.clone(),
                cover_url: cover,
                cover_remote,
                album_id,
            },
            track: String::new(),
            kind: crate::model::RowKind::Song,
            subtitle: row
                .artist
                .clone()
                .or_else(|| library_song.and_then(|song| song.artist.clone()))
                .unwrap_or_default(),
            meta: duration
                .map(|seconds| crate::browse::rows::format_duration(Some(seconds as i32)))
                .unwrap_or_default(),
            song_id,
            starred: library_song
                .and_then(|song| song.starred.as_ref())
                .is_some(),
            rating: 0,
        });
    }

    // A deduplicated payload is also the cache's new truth: persisting it
    // stops the List view from keeping a count the page no longer shows and
    // stops the stale test from refetching forever.
    let shown_duration = if duration_total > 0 {
        duration_total
    } else {
        playlist.duration
    };
    if deduped_locally {
        store_deduped_page_rows(dbs, playlist_id, &rows, shown_duration).await?;
        playlist.song_count = list_rows.len() as i64;
        playlist.duration = shown_duration;
    }

    let origin = if playlist.source == "m3u" {
        "M3U".to_string()
    } else if let Some(owner) = playlist.owner.as_ref().filter(|owner| !owner.is_empty()) {
        owner.clone()
    } else {
        "Playlist".to_string()
    };
    let header = crate::model::PageHeader {
        title: playlist.name.clone(),
        subtitle: format!(
            "{origin} · {} song{}",
            playlist.song_count,
            if playlist.song_count == 1 { "" } else { "s" }
        ),
        meta: if playlist.duration > 0 {
            crate::browse::rows::format_duration(Some(playlist.duration as i32))
        } else {
            String::new()
        },
        cover_url: collage.first().cloned().unwrap_or_default(),
        cover_remote: collage_remote.first().cloned().unwrap_or_default(),
        collage,
        collage_remote,
        collage_art: Vec::new(),
        cover_art: String::new(),
        rating: 0,
        starred: false,
    };
    Ok((header, list_rows))
}

/// Playlist hits for the unified search. Playlists live in their own database,
/// so they are matched here by name rather than through the song FTS index.
pub async fn search_rows(dbs: &Dbs, query: &str, limit: i64) -> Result<Vec<PlaylistHit>> {
    let like = format!("%{}%", query.trim());
    let rows: Vec<PlaylistListRow> = sqlx::query_as(
        "SELECT id, source, name, song_count, duration, owner
         FROM playlist
         WHERE song_count > 0 AND lower(name) LIKE lower(?1)
         ORDER BY lower(name) LIMIT ?2",
    )
    .bind(like)
    .bind(limit.max(1))
    .fetch_all(&dbs.playlist)
    .await?;
    Ok(rows
        .iter()
        .map(|row| PlaylistHit {
            id: row.id.clone(),
            name: row.name.clone(),
            subtitle: format!(
                "{} · {} song{}",
                playlist_origin(row),
                row.song_count,
                if row.song_count == 1 { "" } else { "s" }
            ),
            meta: if row.duration > 0 {
                crate::browse::rows::format_duration(Some(row.duration as i32))
            } else {
                String::new()
            },
        })
        .collect())
}

/// One resolved playlist row (local M3U or server).
struct PlaylistRef {
    id: String,
    name: String,
    source: String,
    path: Option<String>,
}

async fn resolve_playlist(dbs: &Dbs, playlist_query: &str) -> Result<PlaylistRef> {
    let playlist_query = playlist_query.trim();
    let name_key = format!("{playlist_query}.m3u");
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, source, path FROM playlist
         WHERE id = ?1
            OR lower(name) = lower(?2)
            OR lower(name) = lower(?3)
         ORDER BY CASE source WHEN 'm3u' THEN 0 ELSE 1 END
         LIMIT 1",
    )
    .bind(playlist_query)
    .bind(playlist_query)
    .bind(&name_key)
    .fetch_optional(&dbs.playlist)
    .await?;
    let Some((id, name, source, path)) = row else {
        return Err(anyhow!("No playlist matched “{playlist_query}”"));
    };
    Ok(PlaylistRef {
        id,
        name,
        source,
        path,
    })
}

/// Resolve the free-form targets shared by add/remove (`current`, an artist, an
/// album or a song name) to one flat song-id list.
async fn playlist_target_song_ids(
    dbs: &Dbs,
    targets: &[String],
    current_song_id: Option<&str>,
) -> Result<Vec<String>> {
    let mut song_ids = Vec::new();
    for target in targets {
        if target.eq_ignore_ascii_case("current") {
            let current = current_song_id
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow!("Nothing is currently playing."))?;
            song_ids.push(current.to_string());
            continue;
        }
        let ids = crate::download_command::resolve_target_song_ids(dbs, target).await?;
        if ids.is_empty() {
            return Err(anyhow!("“{target}” resolved to no songs"));
        }
        song_ids.extend(ids);
    }
    Ok(song_ids)
}

fn display_playlist_name(name: &str) -> String {
    name.strip_suffix(".m3u")
        .or_else(|| name.strip_suffix(".m3u8"))
        .unwrap_or(name)
        .to_string()
}

fn local_playlist_path(playlist: &PlaylistRef) -> Result<PathBuf> {
    playlist
        .path
        .clone()
        .map(PathBuf::from)
        .or_else(|| Some(m3u_dir().ok()?.join(&playlist.name)))
        .ok_or_else(|| anyhow!("The playlist file path is missing"))
}

/// Add free-form targets to an existing playlist. The final argument resolves
/// by id or name; M3U playlists are edited locally, Subsonic playlists are
/// updated through `updatePlaylist`.
pub async fn add_to_playlist(
    dbs: &Dbs,
    targets: &[String],
    playlist_query: &str,
    current_song_id: Option<&str>,
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    if targets.is_empty() {
        return Err(anyhow!("Select at least one target to add."));
    }
    let song_ids = playlist_target_song_ids(dbs, targets, current_song_id).await?;
    if song_ids.is_empty() {
        return Err(anyhow!("No songs matched the playlist targets."));
    }
    append_resolved(dbs, &playlist, &song_ids).await?;
    Ok(display_playlist_name(&playlist.name))
}

/// Append already-resolved song ids to a playlist (the row-clipboard paste
/// path). Same local/server split as [`add_to_playlist`].
pub async fn add_song_ids_to_playlist(
    dbs: &Dbs,
    playlist_query: &str,
    song_ids: &[String],
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    let song_ids: Vec<String> = song_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if song_ids.is_empty() {
        return Err(anyhow!("No songs selected to add."));
    }
    append_resolved(dbs, &playlist, &song_ids).await?;
    Ok(display_playlist_name(&playlist.name))
}

/// Append the playing queue to a playlist (`playlist add queue; <playlist>`).
///
/// The queue ids are supplied by the caller that owns the playback snapshot
/// (the daemon) or its UI mirror; they are playback state, not a library query.
/// Blank ids are dropped and the resolved-id append path is reused, so
/// duplicates collapse exactly like the other `playlist add` targets.
///
/// Returns the playlist name and the number of tracks the append covers.
pub async fn add_queue_to_playlist(
    dbs: &Dbs,
    playlist_query: &str,
    queue_song_ids: &[String],
) -> Result<(String, usize)> {
    let ids: Vec<String> = queue_song_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return Err(anyhow!("The queue is empty; nothing to save."));
    }
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    let rows = dedup_keep_last(&ids).len();
    append_resolved(dbs, &playlist, &ids).await?;
    Ok((display_playlist_name(&playlist.name), rows))
}

/// Append an already-selected set of rows to a playlist (the
/// `playlist add selected; <playlist>` path). The UI sends its selection as
/// one row kind plus the selected ids: songs are added as-is, albums and
/// artists expand to their songs in selection order, mirroring the row
/// clipboard paste. The playlist resolves by id or name like
/// [`add_to_playlist`]. Returns the playlist name and the number of tracks
/// the append covers.
pub async fn add_selection_to_playlist(
    dbs: &Dbs,
    kind: &str,
    ids: &[String],
    playlist_query: &str,
) -> Result<(String, usize)> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    let ids: Vec<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return Err(anyhow!("No songs selected to add."));
    }
    let songs = crate::queue::quick_songs_many(dbs, kind, &ids).await?;
    let song_ids: Vec<String> = songs.into_iter().map(|song| song.id).collect();
    if song_ids.is_empty() {
        return Err(anyhow!("No songs selected to add."));
    }
    let rows = dedup_keep_last(&song_ids).len();
    append_resolved(dbs, &playlist, &song_ids).await?;
    Ok((display_playlist_name(&playlist.name), rows))
}

async fn append_resolved(dbs: &Dbs, playlist: &PlaylistRef, song_ids: &[String]) -> Result<()> {
    let song_ids = dedup_keep_last(song_ids);
    if song_ids.is_empty() {
        return Ok(());
    }
    if playlist.source == "m3u" {
        let path = local_playlist_path(playlist)?;
        upsert_local_m3u(dbs, &path, usize::MAX, &song_ids).await?;
    } else {
        let client = crate::session::client(dbs)
            .await
            .context("adding to a server playlist needs a signed-in server")?;
        let existing = cached_playlist_song_ids(dbs, &playlist.id).await?;
        let wanted: std::collections::HashSet<&str> = song_ids.iter().map(String::as_str).collect();
        let duplicates: Vec<String> = existing
            .into_iter()
            .filter(|id| wanted.contains(id.as_str()))
            .collect();
        if !duplicates.is_empty() {
            client
                .update_playlist_remove(&playlist.id, &duplicates)
                .await
                .context("clearing the duplicate server entries")?;
        }
        client
            .update_playlist_add(&playlist.id, &song_ids)
            .await
            .context("updating the server playlist")?;
        // The list refresh only carries counts; reload the ordered entries so
        // an open playlist page (and the next one) shows the new songs.
        load_subsonic_songs(dbs, &client, &playlist.id)
            .await
            .context("refreshing the server playlist songs")?;
        refresh_subsonic(dbs, &client)
            .await
            .context("refreshing the server playlist list")?;
    }
    Ok(())
}

/// Remove free-form targets from an existing playlist, mirroring
/// [`add_to_playlist`] (`playlist rm <target...>; <playlist>`). M3U files are
/// rewritten without the matching entries; server playlists use
/// `songIdToRemove`.
pub async fn remove_from_playlist(
    dbs: &Dbs,
    targets: &[String],
    playlist_query: &str,
    current_song_id: Option<&str>,
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    if targets.is_empty() {
        return Err(anyhow!("Select at least one target to remove."));
    }
    let song_ids = playlist_target_song_ids(dbs, targets, current_song_id).await?;
    if song_ids.is_empty() {
        return Err(anyhow!("No songs matched the playlist targets."));
    }

    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        remove_local_m3u(dbs, &path, &song_ids, &playlist.name).await?;
    } else {
        let client = crate::session::client(dbs)
            .await
            .context("removing from a server playlist needs a signed-in server")?;
        client
            .update_playlist_remove(&playlist.id, &song_ids)
            .await
            .context("updating the server playlist")?;
        load_subsonic_songs(dbs, &client, &playlist.id)
            .await
            .context("refreshing the server playlist songs")?;
        refresh_subsonic(dbs, &client)
            .await
            .context("refreshing the server playlist list")?;
    }
    Ok(display_playlist_name(&playlist.name))
}

/// Remove playlist entries by zero-based position (the playlist page `d d`
/// path). Positions are exact for local M3U files; server playlists map the
/// cached positions to song ids and use `songIdToRemove`.
pub async fn remove_playlist_positions(
    dbs: &Dbs,
    playlist_query: &str,
    positions: &[usize],
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    if positions.is_empty() {
        return Err(anyhow!("Select at least one row to delete."));
    }
    let positions: std::collections::HashSet<usize> = positions.iter().copied().collect();

    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        remove_positions_local_m3u(dbs, &path, &positions, &playlist.name).await?;
    } else {
        // The page's positions index the deduplicated view, so map them
        // through the same keep-first cache order instead of raw database
        // positions: a stale cached duplicate must not shift the target id.
        let ordered = cached_playlist_song_ids(dbs, &playlist.id).await?;
        let mut sorted: Vec<usize> = positions.iter().copied().collect();
        sorted.sort_unstable();
        let song_ids: Vec<String> = sorted
            .into_iter()
            .filter_map(|position| ordered.get(position).cloned())
            .collect();
        if song_ids.is_empty() {
            return Err(anyhow!("No playlist entries to remove."));
        }
        let client = crate::session::client(dbs)
            .await
            .context("removing from a server playlist needs a signed-in server")?;
        client
            .update_playlist_remove(&playlist.id, &song_ids)
            .await
            .context("updating the server playlist")?;
        load_subsonic_songs(dbs, &client, &playlist.id)
            .await
            .context("refreshing the server playlist songs")?;
        refresh_subsonic(dbs, &client)
            .await
            .context("refreshing the server playlist list")?;
    }
    Ok(display_playlist_name(&playlist.name))
}

/// Insert or move songs in an app-owned M3U. Existing entries that resolve to
/// an incoming id are dropped before the new copies are written, so the
/// playlist never gains a duplicate (an add is a position update); a stale
/// duplicate already sitting in the file is removed in the same rewrite.
async fn upsert_local_m3u(
    dbs: &Dbs,
    path: &Path,
    position: usize,
    song_ids: &[String],
) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the playlist file {}", path.display()))?;
    let entries = parse_m3u(&text);
    let songs = if entries
        .iter()
        .any(|entry| !entry.path.starts_with("musika:song:"))
    {
        load_songs(&dbs.library).await?
    } else {
        Vec::new()
    };
    let matcher = Matcher::new(&songs);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let incoming = dedup_keep_last(song_ids);
    let wanted: std::collections::HashSet<&str> = incoming.iter().map(String::as_str).collect();
    let mut kept: Vec<M3uEntry> = entries
        .into_iter()
        .filter(|entry| {
            entry_song_id(entry, &matcher, &songs, dir).is_none_or(|id| !wanted.contains(id))
        })
        .collect();
    let at = position.min(kept.len());
    let inserted = m3u_entries_for_ids(dbs, &incoming).await?;
    kept.splice(at..at, inserted);
    let kept = dedup_m3u_entries(kept, &matcher, &songs, dir);
    write_m3u_entries(path, &kept)?;
    scan_dir(dbs, dir).await?;
    Ok(())
}

/// The song id an M3U entry resolves to: an app-owned `musika:song:<id>` line
/// or a local library match. Unresolved path-only entries stay `None`.
fn entry_song_id<'a>(
    entry: &'a M3uEntry,
    matcher: &Matcher,
    songs: &'a [SongRef],
    dir: &Path,
) -> Option<&'a str> {
    if let Some(id) = entry.path.strip_prefix("musika:song:") {
        return Some(id);
    }
    matcher
        .resolve(entry, dir)
        .map(|index| songs[index].id.as_str())
}

/// The duplicate identity of one parsed M3U entry: the resolved song id, or
/// the normalized path of a line that does not resolve to a library song.
fn m3u_entry_identity(
    entry: &M3uEntry,
    matcher: &Matcher,
    songs: &[SongRef],
    dir: &Path,
) -> String {
    match entry_song_id(entry, matcher, songs, dir) {
        Some(id) => format!("id:{id}"),
        None => format!("path:{}", norm(&entry.path)),
    }
}

/// The keep-first cleanup applied to a parsed playlist: a song listed twice
/// keeps its first place, and an unresolved path duplicated under another
/// spelling (`./x.flac` vs `x.flac`) collapses too.
fn dedup_m3u_entries(
    entries: Vec<M3uEntry>,
    matcher: &Matcher,
    songs: &[SongRef],
    dir: &Path,
) -> Vec<M3uEntry> {
    dedup_keep_first(entries, |entry| {
        Some(m3u_entry_identity(entry, matcher, songs, dir))
    })
}

/// [`dedup_m3u_entries`] for a write path that only has the database: loads
/// the library needed to resolve path/name entries (app-owned `musika:song:`
/// lines need no library).
async fn dedup_local_entries(
    dbs: &Dbs,
    dir: &Path,
    entries: Vec<M3uEntry>,
) -> Result<Vec<M3uEntry>> {
    let needs_library = entries
        .iter()
        .any(|entry| !entry.path.starts_with("musika:song:"));
    let songs = if needs_library {
        load_songs(&dbs.library).await?
    } else {
        Vec::new()
    };
    let matcher = Matcher::new(&songs);
    Ok(dedup_m3u_entries(entries, &matcher, &songs, dir))
}

/// Delete a whole playlist (local M3U file or server playlist). Returns the
/// resolved id and display name.
pub async fn delete_playlist(dbs: &Dbs, playlist_query: &str) -> Result<(String, String)> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    let name = display_playlist_name(&playlist.name);
    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        std::fs::remove_file(&path)
            .with_context(|| format!("removing the playlist file {}", path.display()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        scan_dir(dbs, dir).await?;
    } else {
        let client = crate::session::client(dbs)
            .await
            .context("deleting a server playlist needs a signed-in server")?;
        client
            .delete_playlist(&playlist.id)
            .await
            .context("deleting the server playlist")?;
        refresh_subsonic(dbs, &client)
            .await
            .context("refreshing the server playlist list")?;
    }
    Ok((playlist.id, name))
}

/// Insert already-resolved song ids at a playlist position (the playlist page
/// `p`/`P` path). A song already in the playlist is moved instead of copied.
/// Local M3U files keep unresolved neighbours; server playlists are rebuilt in
/// order because `updatePlaylist` has no insert-position call.
pub async fn insert_song_ids_at(
    dbs: &Dbs,
    playlist_query: &str,
    position: usize,
    song_ids: &[String],
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    let song_ids: Vec<String> = song_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    let song_ids = dedup_keep_last(&song_ids);
    if song_ids.is_empty() {
        return Err(anyhow!("No songs selected to paste."));
    }
    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        upsert_local_m3u(dbs, &path, position, &song_ids).await?;
    } else {
        let mut ordered = cached_playlist_song_ids(dbs, &playlist.id).await?;
        upsert_ids(&mut ordered, position, &song_ids);
        rebuild_server_playlist(dbs, &playlist, &ordered).await?;
    }
    Ok(display_playlist_name(&playlist.name))
}

/// Move one playlist entry to another position (the playlist page
/// `Ctrl-Shift-J/K` path).
pub async fn move_playlist_entry(
    dbs: &Dbs,
    playlist_query: &str,
    from: usize,
    to: usize,
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading the playlist file {}", path.display()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        // A stale file may still hold duplicates: clean them first so the
        // positions (which come from the deduplicated page) line up and the
        // move never writes a duplicate back.
        let mut entries = dedup_local_entries(dbs, dir, parse_m3u(&text)).await?;
        if from >= entries.len() || to >= entries.len() {
            return Err(anyhow!("The playlist row is out of date."));
        }
        let entry = entries.remove(from);
        entries.insert(to, entry);
        write_m3u_entries(&path, &entries)?;
        scan_dir(dbs, dir).await?;
    } else {
        let mut ordered = cached_playlist_song_ids(dbs, &playlist.id).await?;
        if from >= ordered.len() || to >= ordered.len() {
            return Err(anyhow!("The playlist row is out of date."));
        }
        let id = ordered.remove(from);
        ordered.insert(to, id);
        rebuild_server_playlist(dbs, &playlist, &ordered).await?;
    }
    Ok(display_playlist_name(&playlist.name))
}

/// Move a contiguous block of playlist rows by `delta` positions (visual
/// selection reorder). Local M3U files splice the block; server playlists use
/// the same remove/re-add order rebuild as single moves.
pub async fn move_playlist_block(
    dbs: &Dbs,
    playlist_query: &str,
    start: usize,
    len: usize,
    delta: i32,
) -> Result<String> {
    let playlist = resolve_playlist(dbs, playlist_query).await?;
    if len == 0 || delta == 0 {
        return Err(anyhow!("Nothing to move."));
    }
    if playlist.source == "m3u" {
        let path = local_playlist_path(&playlist)?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading the playlist file {}", path.display()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut entries = dedup_local_entries(dbs, dir, parse_m3u(&text)).await?;
        let end = (start + len).min(entries.len());
        let block_len = end.saturating_sub(start);
        let new_start = start as i64 + i64::from(delta);
        if block_len == 0
            || new_start < 0
            || new_start as usize + block_len > entries.len()
            || new_start as usize == start
        {
            return Err(anyhow!("The playlist row is out of date."));
        }
        let new_start = new_start as usize;
        let block: Vec<M3uEntry> = entries.drain(start..end).collect();
        entries.splice(new_start..new_start, block);
        write_m3u_entries(&path, &entries)?;
        scan_dir(dbs, dir).await?;
    } else {
        let mut ordered = cached_playlist_song_ids(dbs, &playlist.id).await?;
        let end = (start + len).min(ordered.len());
        let block_len = end.saturating_sub(start);
        let new_start = start as i64 + i64::from(delta);
        if block_len == 0
            || new_start < 0
            || new_start as usize + block_len > ordered.len()
            || new_start as usize == start
        {
            return Err(anyhow!("The playlist row is out of date."));
        }
        let new_start = new_start as usize;
        let block: Vec<String> = ordered.drain(start..end).collect();
        ordered.splice(new_start..new_start, block);
        rebuild_server_playlist(dbs, &playlist, &ordered).await?;
    }
    Ok(display_playlist_name(&playlist.name))
}

/// The cached ordered song ids of a playlist (server rows only; M3U entries
/// always resolve through the file).
async fn cached_playlist_song_ids(dbs: &Dbs, playlist_id: &str) -> Result<Vec<String>> {
    let rows: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT song_id FROM playlist_song WHERE playlist_id = ?1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(&dbs.playlist)
    .await?;
    let ids: Vec<String> = rows.into_iter().flatten().collect();
    // Edit operations rebuild the server playlist from this order; a stale
    // cache holding a duplicate must not be written back as a duplicate.
    Ok(dedup_keep_first_ids(&ids))
}

/// Parse the M3U body for `song_ids` back into entries for local insertion.
async fn m3u_entries_for_ids(dbs: &Dbs, song_ids: &[String]) -> Result<Vec<M3uEntry>> {
    let body = m3u_text(dbs, song_ids).await?;
    Ok(parse_m3u(&body))
}

fn write_m3u_entries(path: &Path, entries: &[M3uEntry]) -> Result<()> {
    let mut out = String::from("#EXTM3U\n");
    for entry in entries {
        push_m3u_entry(&mut out, entry);
    }
    std::fs::write(path, out)
        .with_context(|| format!("writing the playlist file {}", path.display()))?;
    Ok(())
}

/// Rebuild a server playlist in the given order. The API can only append and
/// remove, so removing every cached id and re-adding the new order is the only
/// way to insert/move; the ordered entries are reloaded from the server after,
/// which also cleans any duplicate the server still reported.
async fn rebuild_server_playlist(
    dbs: &Dbs,
    playlist: &PlaylistRef,
    ordered_ids: &[String],
) -> Result<()> {
    let client = crate::session::client(dbs)
        .await
        .context("editing a server playlist needs a signed-in server")?;
    let current = cached_playlist_song_ids(dbs, &playlist.id).await?;
    // The cache is already deduplicated, but treat the target order the same
    // way: a move/insert must never write a duplicate back to the server.
    let ordered_ids = dedup_keep_first_ids(ordered_ids);
    if !current.is_empty() {
        client
            .update_playlist_remove(&playlist.id, &current)
            .await
            .context("clearing the server playlist order")?;
    }
    if !ordered_ids.is_empty() {
        client
            .update_playlist_add(&playlist.id, &ordered_ids)
            .await
            .context("writing the server playlist order")?;
    }
    load_subsonic_songs(dbs, &client, &playlist.id)
        .await
        .context("refreshing the server playlist songs")?;
    refresh_subsonic(dbs, &client)
        .await
        .context("refreshing the server playlist list")?;
    Ok(())
}

/// Rewrite an app-owned M3U without entries resolving to `remove_ids`, then
/// rescan the folder. Unresolved entries (plain path lines) are preserved.
async fn remove_local_m3u(
    dbs: &Dbs,
    path: &Path,
    remove_ids: &[String],
    playlist_name: &str,
) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the playlist file {}", path.display()))?;
    let entries = parse_m3u(&text);
    let songs = load_songs(&dbs.library).await?;
    let matcher = Matcher::new(&songs);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let entries = dedup_m3u_entries(entries, &matcher, &songs, dir);
    let remove: std::collections::HashSet<&str> = remove_ids.iter().map(String::as_str).collect();

    let mut out = String::from("#EXTM3U\n");
    let mut removed = 0usize;
    for entry in &entries {
        let song_id = matcher
            .resolve(entry, dir)
            .map(|index| songs[index].id.as_str());
        if song_id.is_some_and(|id| remove.contains(id)) {
            removed += 1;
            continue;
        }
        push_m3u_entry(&mut out, entry);
    }
    if removed == 0 {
        return Err(anyhow!("None of the targets are in “{playlist_name}”"));
    }
    std::fs::write(path, out)
        .with_context(|| format!("writing the playlist file {}", path.display()))?;
    scan_dir(dbs, dir).await?;
    Ok(())
}

/// Rewrite an app-owned M3U without the entries at `positions` (0-based order
/// of the deduplicated parsed file). Duplicates in a stale file are removed in
/// the same rewrite so they cannot survive an edit.
async fn remove_positions_local_m3u(
    dbs: &Dbs,
    path: &Path,
    positions: &std::collections::HashSet<usize>,
    playlist_name: &str,
) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the playlist file {}", path.display()))?;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    // The positions come from the deduplicated page, so clean the file first
    // and index the same order the user saw.
    let entries = dedup_local_entries(dbs, dir, parse_m3u(&text)).await?;

    let mut out = String::from("#EXTM3U\n");
    let mut removed = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        if positions.contains(&index) {
            removed += 1;
            continue;
        }
        push_m3u_entry(&mut out, entry);
    }
    if removed == 0 {
        return Err(anyhow!("No matching rows in “{playlist_name}”"));
    }
    std::fs::write(path, out)
        .with_context(|| format!("writing the playlist file {}", path.display()))?;
    scan_dir(dbs, dir).await?;
    Ok(())
}

/// Re-emit one parsed M3U entry, keeping its EXTINF display and path.
fn push_m3u_entry(out: &mut String, entry: &M3uEntry) {
    let display = match (&entry.artist, &entry.title) {
        (Some(artist), Some(title)) if !artist.is_empty() && !title.is_empty() => {
            Some(format!("{artist} - {title}"))
        }
        (_, Some(title)) if !title.is_empty() => Some(title.clone()),
        _ => None,
    };
    if let (Some(duration), Some(display)) = (entry.duration, display) {
        out.push_str(&format!("#EXTINF:{duration},{}\n", m3u_line(&display)));
    }
    out.push_str(&m3u_line(&entry.path));
    out.push('\n');
}
