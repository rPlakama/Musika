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

//! Lyrics pipeline.
//!
//! Resolution order:
//!
//!   local override (`~/.config/musika/lyrics/<song_id>.lrc|.txt`)
//!     → OpenSubsonic `getLyricsBySongId` (per-song, optionally synced)
//!     → legacy `getLyrics` (artist/title, plain)
//!
//! Everything lands in `lyrics.db` (DB5). Misses are cached as
//! `source = 'none'` with a TTL so opening the same song twice does not
//! hammer the server; a failed *request* is never cached as a miss.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use crate::api::LyricsList;
use crate::consts::now_secs;
use crate::db::Dbs;
use crate::http_client::SubsonicClient;

/// How long a cached miss is trusted before asking the server again.
pub(crate) const MISS_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// How many missing/expired songs one sync pass will fetch lyrics for.
pub const SYNC_BACKFILL_LIMIT: usize = 64;

/// Bounded concurrency for the sync backfill (mirrors the index fetchers).

/// How long a cached *plain* document is trusted: Navidrome may gain a `.lrc`
/// for a track later, so an unsynced hit must be revalidated eventually.
pub(crate) const UNSYNCED_TTL_SECS: i64 = 24 * 60 * 60;

/// A cached (or freshly fetched) lyric document.
#[derive(Debug, Clone)]
pub struct Document {
    pub song_id: String,
    /// `lrc` | `subsonic` | `legacy` | `none`.
    pub source: String,
    pub synced: bool,
    pub lang: Option<String>,
    pub offset_ms: i64,
    pub text: String,
    pub lines: Vec<Line>,
}

/// One rendered lyric line; `start_ms` is `None` for plain lyrics.
#[derive(Debug, Clone)]
pub struct Line {
    pub start_ms: Option<i64>,
    pub value: String,
}

fn lines_to_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.value.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_to_lines(text: &str) -> Vec<Line> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| Line {
            start_ms: None,
            value: line.to_string(),
        })
        .collect()
}

impl Document {
    pub fn missing(song_id: &str) -> Self {
        Self {
            song_id: song_id.to_string(),
            source: "none".to_string(),
            synced: false,
            lang: None,
            offset_ms: 0,
            text: String::new(),
            lines: Vec::new(),
        }
    }

    /// The payload sent to QML over the `lyrics` dataReady channel.
    pub fn to_json(&self) -> String {
        let lines: Vec<serde_json::Value> = self
            .lines
            .iter()
            .map(|line| serde_json::json!({ "t": line.start_ms, "text": line.value }))
            .collect();
        serde_json::json!({
            "song_id": self.song_id,
            "source": self.source,
            "synced": self.synced,
            "lang": self.lang,
            "offset_ms": self.offset_ms,
            "lines": lines,
        })
        .to_string()
    }

    pub(crate) fn plain(song_id: &str, source: &str, text: &str) -> Self {
        let lines = text_to_lines(text);
        Self {
            song_id: song_id.to_string(),
            source: source.to_string(),
            synced: false,
            lang: None,
            offset_ms: 0,
            text: text.trim().to_string(),
            lines,
        }
    }
}

/// Legacy `getLyrics` text: parsed as LRC when it carries timestamps, kept
/// plain otherwise. Servers frequently answer the legacy endpoint with the
/// raw `.lrc` file, and dropping those tags would render it unsynced.
pub(crate) fn legacy_document(song_id: &str, value: &str) -> Document {
    let (synced, lines) = parse_lrc(value);
    if !synced {
        return Document::plain(song_id, "legacy", value);
    }

    let text = lines_to_text(&lines);
    Document {
        song_id: song_id.to_string(),
        source: "legacy".to_string(),
        synced,
        lang: None,
        offset_ms: 0,
        text,
        lines,
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Resolves lyrics for `song_id`, consulting the override, the cache and the
/// server (when `client` is available), in that order.
pub async fn fetch(dbs: &Dbs, client: Option<&SubsonicClient>, song_id: &str) -> Result<Document> {
    if let Some(document) = override_document(song_id) {
        return Ok(document);
    }

    if let Some((document, fetched_at)) = load_cached(&dbs.lyrics, song_id).await? {
        if !cache_is_stale(&document, fetched_at) {
            return Ok(document);
        }
    }

    let Some(client) = client else {
        return Ok(Document::missing(song_id));
    };

    let mut errored = false;

    // Per-song OpenSubsonic endpoint, when the server advertises it.
    if song_lyrics_supported(dbs, client).await {
        match client.get_lyrics_by_song_id(song_id).await {
            Ok(list) => {
                if let Some(document) = structured_document(song_id, &list) {
                    store(&dbs.lyrics, &document).await?;
                    return Ok(document);
                }
            }
            Err(err) => {
                errored = true;
                eprintln!("lyrics: getLyricsBySongId failed for {song_id}: {err:#}");
            }
        }
    }

    // Legacy fallback: artist/title match. Some servers answer the legacy
    // endpoint with the raw `.lrc`, so timestamps are parsed when present.
    if let Some(song) = crate::library::song(&dbs.library, song_id).await? {
        match client
            .get_lyrics(song.artist.as_deref(), Some(song.title.as_str()))
            .await
        {
            Ok(legacy) if !legacy.value.trim().is_empty() => {
                let document = legacy_document(song_id, &legacy.value);
                store(&dbs.lyrics, &document).await?;
                return Ok(document);
            }
            Ok(_) => {}
            Err(err) => {
                errored = true;
                eprintln!("lyrics: legacy getLyrics failed for {song_id}: {err:#}");
            }
        }
    }

    let document = Document::missing(song_id);
    if !errored {
        store(&dbs.lyrics, &document).await?;
    }
    Ok(document)
}

/// Outcome of one bounded sync backfill pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillReport {
    /// Candidates that ended up with real lyrics.
    pub fetched: usize,
}

/// Fetches lyrics for songs that have no cached row, or whose miss TTL
/// expired. Bounded and best-effort: one failed song never aborts the pass.
///
/// This is the sync button's lyric pass — the only place lyrics are fetched
/// outside playback. It is capped so a big library is topped up over several
/// syncs instead of one request storm.
pub async fn backfill(
    dbs: &Dbs,
    client: &Arc<SubsonicClient>,
    limit: usize,
) -> Result<BackfillReport> {
    let songs = crate::library::song_ids(&dbs.library).await?;
    if songs.is_empty() {
        return Ok(BackfillReport::default());
    }

    let rows = cached_rows(&dbs.lyrics).await?;
    let candidates = candidates(&songs, &rows, now_secs(), limit);
    let mut report = BackfillReport { fetched: 0 };

    let dbs = dbs.clone();
    let client = Arc::clone(client);
    let results = crate::fanout::map_bounded(candidates, move |song_id| {
        let dbs = dbs.clone();
        let client = Arc::clone(&client);
        async move { fetch(&dbs, Some(client.as_ref()), &song_id).await }
    })
    .await;

    for (id, result) in results {
        match result {
            Ok(document) => {
                if document.source != "none" {
                    report.fetched += 1;
                }
            }
            Err(err) => eprintln!("lyrics: sync backfill failed for {id}: {err:#}"),
        }
    }

    Ok(report)
}

async fn cached_rows(pool: &SqlitePool) -> Result<Vec<(String, String, i64)>> {
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT song_id, source, fetched_at FROM lyrics",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Songs worth (re)fetching: no row at all, or a miss past its TTL.
pub(crate) fn candidates(
    songs: &[String],
    rows: &[(String, String, i64)],
    now: i64,
    limit: usize,
) -> Vec<String> {
    use std::collections::HashMap;

    let known: HashMap<&str, (&str, i64)> = rows
        .iter()
        .map(|(id, source, fetched_at)| (id.as_str(), (source.as_str(), *fetched_at)))
        .collect();

    songs
        .iter()
        .filter(|id| match known.get(id.as_str()) {
            None => true,
            Some((source, fetched_at)) => *source == "none" && now - *fetched_at > MISS_TTL_SECS,
        })
        .take(limit)
        .cloned()
        .collect()
}

/// Deletes cached lyrics whose song left the library.
pub async fn purge(dbs: &Dbs) -> Result<usize> {
    purge_with(&dbs.lyrics, &dbs.library).await
}

/// Drops the cached document for one song, forcing the next fetch to hit the
/// server again (manual `lyrics_refresh`; LYR-5).
pub async fn invalidate(lyrics: &SqlitePool, song_id: &str) -> Result<()> {
    crate::cache::delete_by(lyrics, "DELETE FROM lyrics WHERE song_id = ?1", song_id).await
}

/// Drops cached lyrics fetched before the provider's last library update:
/// the provider has newer data, so the old documents must not win.
pub async fn invalidate_before(lyrics: &SqlitePool, before: i64) -> Result<u64> {
    let mut tx = lyrics.begin().await?;
    sqlx::query(
        "DELETE FROM lyric_line
         WHERE song_id IN (SELECT song_id FROM lyrics WHERE fetched_at < ?1)",
    )
    .bind(before)
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query("DELETE FROM lyrics WHERE fetched_at < ?1")
        .bind(before)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(result.rows_affected())
}

/// Drops cached lyrics for songs the API just changed, so the next play and
/// the sync backfill fetch them again.
pub async fn invalidate_songs(lyrics: &SqlitePool, song_ids: &[String]) -> Result<()> {
    if song_ids.is_empty() {
        return Ok(());
    }
    let mut tx = lyrics.begin().await?;
    crate::cache::delete_in(
        &mut *tx,
        "DELETE FROM lyric_line WHERE song_id IN (",
        song_ids,
    )
    .await?;
    crate::cache::delete_in(&mut *tx, "DELETE FROM lyrics WHERE song_id IN (", song_ids).await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Store / load
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct CachedRow {
    song_id: String,
    source: String,
    synced: i64,
    lang: Option<String>,
    offset_ms: i64,
    text: String,
    fetched_at: i64,
}

pub(crate) async fn load_cached(
    pool: &SqlitePool,
    song_id: &str,
) -> Result<Option<(Document, i64)>> {
    let row = sqlx::query_as::<_, CachedRow>(
        "SELECT song_id, source, synced, lang, offset_ms, text, fetched_at
         FROM lyrics WHERE song_id = ?1",
    )
    .bind(song_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    let synced = row.synced != 0;
    let lines = if synced {
        sqlx::query_as::<_, (Option<i64>, String)>(
            "SELECT start_ms, value FROM lyric_line WHERE song_id = ?1 ORDER BY idx",
        )
        .bind(song_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(start_ms, value)| Line { start_ms, value })
        .collect()
    } else {
        text_to_lines(&row.text)
    };

    Ok(Some((
        Document {
            song_id: row.song_id,
            source: row.source,
            synced,
            lang: row.lang,
            offset_ms: row.offset_ms,
            text: row.text,
            lines,
        },
        row.fetched_at,
    )))
}

pub(crate) async fn store(pool: &SqlitePool, document: &Document) -> Result<()> {
    let mut tx = pool.begin().await?;

    crate::cache::delete_by(
        &mut *tx,
        "DELETE FROM lyric_line WHERE song_id = ?1",
        &document.song_id,
    )
    .await?;

    sqlx::query(
        "INSERT INTO lyrics (song_id, source, synced, lang, offset_ms, text, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(song_id) DO UPDATE SET
             source = excluded.source,
             synced = excluded.synced,
             lang = excluded.lang,
             offset_ms = excluded.offset_ms,
             text = excluded.text,
             fetched_at = excluded.fetched_at",
    )
    .bind(&document.song_id)
    .bind(&document.source)
    .bind(document.synced as i64)
    .bind(&document.lang)
    .bind(document.offset_ms)
    .bind(&document.text)
    .bind(now_secs())
    .execute(&mut *tx)
    .await?;

    if document.synced {
        for (idx, line) in document.lines.iter().enumerate() {
            sqlx::query(
                "INSERT INTO lyric_line (song_id, idx, start_ms, value) VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(&document.song_id)
            .bind(idx as i64)
            .bind(line.start_ms)
            .bind(&line.value)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Removes lyric rows whose song id is absent from `library`.
pub async fn purge_with(lyrics: &SqlitePool, library: &SqlitePool) -> Result<usize> {
    // Only the cached lyric ids are queried against `songs`, never the whole
    // catalog (a 100k-song library is much larger than the lyric cache).
    crate::cache::purge_missing_against(lyrics, "lyrics", "song_id", library, "songs", "id").await
}

// ---------------------------------------------------------------------------
// Capability probe
// ---------------------------------------------------------------------------

/// Whether the server advertises the OpenSubsonic `songLyrics` extension.
///
/// The answer is cached in `lyrics_meta`; when the probe itself fails we assume
/// support and let `getLyricsBySongId` decide.
async fn song_lyrics_supported(dbs: &Dbs, client: &SubsonicClient) -> bool {
    if let Ok(Some(value)) = meta_get(&dbs.lyrics, "song_lyrics").await {
        return value == "yes";
    }

    match client.get_open_subsonic_extensions().await {
        Ok(extensions) => {
            let supported = extensions
                .iter()
                .any(|extension| extension.name == "songLyrics" && !extension.versions.is_empty());
            let _ = meta_set(
                &dbs.lyrics,
                "song_lyrics",
                if supported { "yes" } else { "no" },
            )
            .await;
            supported
        }
        Err(err) => {
            eprintln!("lyrics: could not probe OpenSubsonic extensions: {err:#}");
            true
        }
    }
}

async fn meta_get(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM lyrics_meta WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value)
}

async fn meta_set(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Local override
// ---------------------------------------------------------------------------

/// `<config>/lyrics/<song_id>.lrc` (synced) or `.txt` (plain) wins over the
/// server and is never cached or purged.
fn override_document(song_id: &str) -> Option<Document> {
    let safe = sanitize_id(song_id)?;
    let dir = override_dir()?;

    for extension in ["lrc", "txt"] {
        let path = dir.join(format!("{safe}.{extension}"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }

        if extension == "lrc" {
            let (synced, lines) = parse_lrc(&text);
            let plain = lines_to_text(&lines);
            return Some(Document {
                song_id: song_id.to_string(),
                source: "lrc".to_string(),
                synced,
                lang: None,
                offset_ms: 0,
                text: plain,
                lines,
            });
        }

        return Some(Document::plain(song_id, "lrc", &text));
    }

    None
}

fn override_dir() -> Option<PathBuf> {
    crate::db::config_dir().ok().map(|dir| dir.join("lyrics"))
}

/// Keeps the override file name safe: ids with separators are ignored.
fn sanitize_id(song_id: &str) -> Option<String> {
    if song_id.is_empty() {
        return None;
    }
    let safe: String = song_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    (safe.len() == song_id.len()).then_some(safe)
}

/// Parses LRC text into `(synced, lines)`. Metadata tags (`[ar:…]`) are
/// skipped; a line may carry several timestamps.
pub(crate) fn parse_lrc(text: &str) -> (bool, Vec<Line>) {
    let mut lines = Vec::new();
    let mut synced = false;

    for raw in text.lines() {
        let mut rest = raw.trim_end();
        let mut starts = Vec::new();

        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            if let Some(start) = parse_timestamp(&rest[1..end]) {
                starts.push(start);
                synced = true;
            }
            rest = &rest[end + 1..];
        }

        let value = rest.trim();
        if value.is_empty() {
            continue;
        }

        if starts.is_empty() {
            lines.push(Line {
                start_ms: None,
                value: value.to_string(),
            });
        } else {
            for start in starts {
                lines.push(Line {
                    start_ms: Some(start),
                    value: value.to_string(),
                });
            }
        }
    }

    (synced, lines)
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx` or `mm:ss.xxx` to milliseconds.
fn parse_timestamp(tag: &str) -> Option<i64> {
    let (minutes, rest) = tag.split_once(':')?;
    let minutes: i64 = minutes.trim().parse().ok()?;

    let (seconds, fraction) = match rest.split_once('.') {
        Some((seconds, fraction)) => (seconds, Some(fraction)),
        None => (rest, None),
    };
    let seconds: i64 = seconds.trim().parse().ok()?;

    let millis = match fraction {
        Some(fraction) => {
            let digits: String = fraction.chars().take(3).collect();
            let value: i64 = digits.parse().ok()?;
            match digits.len() {
                1 => value * 100,
                2 => value * 10,
                _ => value,
            }
        }
        None => 0,
    };

    Some(minutes * 60_000 + seconds * 1_000 + millis)
}

// ---------------------------------------------------------------------------
// Server payloads
// ---------------------------------------------------------------------------

/// Picks a structured entry (synced preferred) and normalizes it.
pub(crate) fn structured_document(song_id: &str, list: &LyricsList) -> Option<Document> {
    let entry = list
        .structured_lyrics
        .iter()
        .find(|entry| entry.synced && !entry.line.is_empty())
        .or_else(|| {
            list.structured_lyrics
                .iter()
                .find(|entry| !entry.line.is_empty())
        })?;

    let mut lines: Vec<Line> = entry
        .line
        .iter()
        .filter(|line| !line.value.trim().is_empty())
        .map(|line| Line {
            start_ms: line.start,
            value: line.value.clone(),
        })
        .collect();

    if lines.is_empty() {
        return None;
    }

    // Trust the line data over the server flag: some providers mark the whole
    // entry unsynced while still sending `start` on every line. When the flag
    // says synced but a line lacks a timestamp, the known starts are kept and
    // the untimed line simply never activates.
    let all_timed = lines.iter().all(|line| line.start_ms.is_some());
    let synced = all_timed || entry.synced;
    if !synced {
        for line in &mut lines {
            line.start_ms = None;
        }
    }

    let text = lines_to_text(&lines);

    Some(Document {
        song_id: song_id.to_string(),
        source: "subsonic".to_string(),
        synced,
        lang: entry.lang.clone(),
        offset_ms: entry.offset.unwrap_or(0),
        text,
        lines,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn miss_expired(fetched_at: i64) -> bool {
    now_secs() - fetched_at > MISS_TTL_SECS
}

/// Whether a cache hit must be re-requested. Synced documents are trusted
/// forever (songs carry no server-side "changed" signal to revalidate
/// against); misses and plain documents expire so a `.lrc` added later is
/// picked up.
pub(crate) fn cache_is_stale(document: &Document, fetched_at: i64) -> bool {
    if document.synced {
        return false;
    }
    if document.source == "none" {
        return miss_expired(fetched_at);
    }
    now_secs() - fetched_at > UNSYNCED_TTL_SECS
}
