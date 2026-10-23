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
const MISS_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// How many missing/expired songs one sync pass will fetch lyrics for.
pub const SYNC_BACKFILL_LIMIT: usize = 64;

/// Bounded concurrency for the sync backfill (mirrors the index fetchers).

/// How long a cached *plain* document is trusted: Navidrome may gain a `.lrc`
/// for a track later, so an unsynced hit must be revalidated eventually.
const UNSYNCED_TTL_SECS: i64 = 24 * 60 * 60;

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

    fn plain(song_id: &str, source: &str, text: &str) -> Self {
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
fn legacy_document(song_id: &str, value: &str) -> Document {
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
fn candidates(
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

async fn load_cached(pool: &SqlitePool, song_id: &str) -> Result<Option<(Document, i64)>> {
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

async fn store(pool: &SqlitePool, document: &Document) -> Result<()> {
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
    let existing: std::collections::HashSet<String> = sqlx::query_scalar("SELECT id FROM songs")
        .fetch_all(library)
        .await?
        .into_iter()
        .collect();
    crate::cache::purge_missing(lyrics, "lyrics", "song_id", &existing).await
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
fn parse_lrc(text: &str) -> (bool, Vec<Line>) {
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
fn structured_document(song_id: &str, list: &LyricsList) -> Option<Document> {
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

fn miss_expired(fetched_at: i64) -> bool {
    now_secs() - fetched_at > MISS_TTL_SECS
}

/// Whether a cache hit must be re-requested. Synced documents are trusted
/// forever (songs carry no server-side "changed" signal to revalidate
/// against); misses and plain documents expire so a `.lrc` added later is
/// picked up.
fn cache_is_stale(document: &Document, fetched_at: i64) -> bool {
    if document.synced {
        return false;
    }
    if document.source == "none" {
        return miss_expired(fetched_at);
    }
    now_secs() - fetched_at > UNSYNCED_TTL_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{LyricLine, StructuredLyrics};

    async fn lyrics_pool() -> SqlitePool {
        crate::sqlite::memory_pool(sqlx::migrate!("./migrations_lyrics")).await
    }

    async fn library_pool() -> SqlitePool {
        crate::sqlite::memory_pool(None).await
    }

    #[test]
    fn lrc_parsing_keeps_timestamps_and_skips_metadata() {
        let text = "[ar:Radiohead]\n[00:12.34]First line\n[01:02.5][01:04.00]Twice";
        let (synced, lines) = parse_lrc(text);
        assert!(synced);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_ms, Some(12_340));
        assert_eq!(lines[0].value, "First line");
        assert_eq!(lines[1].start_ms, Some(62_500));
        assert_eq!(lines[2].start_ms, Some(64_000));
    }

    #[test]
    fn plain_text_without_timestamps_is_unsynced() {
        let (synced, lines) = parse_lrc("just words\nno timing here");
        assert!(!synced);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.start_ms.is_none()));
    }

    #[test]
    fn structured_selection_prefers_synced_and_normalizes() {
        let list = LyricsList {
            structured_lyrics: vec![
                StructuredLyrics {
                    synced: false,
                    line: vec![LyricLine {
                        start: None,
                        value: "plain".to_string(),
                    }],
                    ..StructuredLyrics::default()
                },
                StructuredLyrics {
                    synced: true,
                    offset: Some(250),
                    line: vec![
                        LyricLine {
                            start: Some(1_000),
                            value: "first".to_string(),
                        },
                        LyricLine {
                            start: Some(2_000),
                            value: "second".to_string(),
                        },
                    ],
                    ..StructuredLyrics::default()
                },
            ],
        };

        let document = structured_document("s1", &list).unwrap();
        assert_eq!(document.source, "subsonic");
        assert!(document.synced);
        assert_eq!(document.offset_ms, 250);
        assert_eq!(document.lines.len(), 2);
        assert_eq!(document.lines[1].start_ms, Some(2_000));
    }

    #[test]
    fn partially_timed_entries_keep_their_known_starts() {
        let list = LyricsList {
            structured_lyrics: vec![StructuredLyrics {
                synced: true,
                line: vec![
                    LyricLine {
                        start: Some(1_000),
                        value: "timed".to_string(),
                    },
                    LyricLine {
                        start: None,
                        value: "untimed".to_string(),
                    },
                ],
                ..StructuredLyrics::default()
            }],
        };

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines[0].start_ms, Some(1_000));
        assert_eq!(document.lines[1].start_ms, None);
    }

    #[test]
    fn server_unsynced_flag_does_not_hide_line_timing() {
        // Some providers mark the entry unsynced while still sending `start`
        // on every line; the data wins over the flag.
        let list = LyricsList {
            structured_lyrics: vec![StructuredLyrics {
                synced: false,
                line: vec![
                    LyricLine {
                        start: Some(0),
                        value: "one".to_string(),
                    },
                    LyricLine {
                        start: Some(5_990),
                        value: "two".to_string(),
                    },
                ],
                ..StructuredLyrics::default()
            }],
        };

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines[1].start_ms, Some(5_990));
    }

    #[test]
    fn navidrome_synced_payload_is_not_downgraded() {
        // Trimmed getLyricsBySongId payload: synced true, empty-value lines
        // between the lyrics (the live probe for "Muscle Museum").
        let payload = serde_json::json!({
            "structuredLyrics": [{
                "displayArtist": "Muse",
                "displayTitle": "Muscle Museum",
                "lang": "xxx",
                "synced": true,
                "line": [
                    { "start": 0, "value": "Muscle Museum (soulwax remix)" },
                    { "start": 4840, "value": "" },
                    { "start": 5990, "value": "She had something to confess to" },
                    { "start": 10250, "value": "But you don’t have the time so" }
                ]
            }]
        });
        let list: LyricsList = serde_json::from_value(payload).unwrap();

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines.len(), 3, "empty lines are dropped");
        assert_eq!(document.lines[1].start_ms, Some(5_990));
        assert_eq!(document.lang.as_deref(), Some("xxx"));
    }

    #[test]
    fn legacy_lrc_text_is_parsed_when_timed() {
        let document = legacy_document("s1", "[00:01.00]first\n[00:02.50]second");
        assert_eq!(document.source, "legacy");
        assert!(document.synced);
        assert_eq!(document.lines[0].start_ms, Some(1_000));
        assert_eq!(document.lines[1].start_ms, Some(2_500));

        let plain = legacy_document("s2", "just words\nno timing here");
        assert!(!plain.synced);
        assert!(plain.lines.iter().all(|line| line.start_ms.is_none()));
    }

    #[test]
    fn unsynced_cache_expires_but_synced_does_not() {
        let old = now_secs() - UNSYNCED_TTL_SECS - 1;
        let fresh = now_secs();

        assert!(cache_is_stale(&Document::plain("s", "legacy", "a"), old));
        assert!(!cache_is_stale(&Document::plain("s", "legacy", "a"), fresh));
        assert!(cache_is_stale(
            &Document::missing("s"),
            now_secs() - MISS_TTL_SECS - 1
        ));
        assert!(!cache_is_stale(&Document::missing("s"), fresh));

        let mut synced = Document::plain("s", "subsonic", "a");
        synced.synced = true;
        assert!(!cache_is_stale(&synced, old), "synced hits never expire");
    }

    #[tokio::test]
    async fn synced_documents_round_trip_through_the_cache() {
        let pool = lyrics_pool().await;
        let document = Document {
            song_id: "s1".to_string(),
            source: "subsonic".to_string(),
            synced: true,
            lang: Some("eng".to_string()),
            offset_ms: 100,
            text: "one\ntwo".to_string(),
            lines: vec![
                Line {
                    start_ms: Some(500),
                    value: "one".to_string(),
                },
                Line {
                    start_ms: Some(1_500),
                    value: "two".to_string(),
                },
            ],
        };

        store(&pool, &document).await.unwrap();
        let (loaded, fetched_at) = load_cached(&pool, "s1").await.unwrap().unwrap();

        assert_eq!(loaded.source, "subsonic");
        assert!(loaded.synced);
        assert_eq!(loaded.offset_ms, 100);
        assert_eq!(loaded.lines.len(), 2);
        assert_eq!(loaded.lines[0].start_ms, Some(500));
        assert!(fetched_at > 0);
    }

    #[tokio::test]
    async fn plain_documents_round_trip_and_misses_too() {
        let pool = lyrics_pool().await;

        let document = Document::plain("s2", "legacy", "alpha\n\nbeta");
        store(&pool, &document).await.unwrap();
        let (loaded, _) = load_cached(&pool, "s2").await.unwrap().unwrap();
        assert!(!loaded.synced);
        assert_eq!(loaded.lines.len(), 2);
        assert_eq!(loaded.lines[1].value, "beta");

        let miss = Document::missing("s3");
        store(&pool, &miss).await.unwrap();
        let (loaded, _) = load_cached(&pool, "s3").await.unwrap().unwrap();
        assert_eq!(loaded.source, "none");
        assert!(!miss_expired(now_secs()));
    }

    #[tokio::test]
    async fn invalidate_drops_the_cached_document() {
        let pool = lyrics_pool().await;
        store(&pool, &Document::plain("s9", "subsonic", "one\ntwo"))
            .await
            .unwrap();

        invalidate(&pool, "s9").await.unwrap();
        assert!(load_cached(&pool, "s9").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalidate_songs_drops_only_the_changed_documents() {
        let pool = lyrics_pool().await;
        for id in ["s1", "s2", "s3"] {
            store(&pool, &Document::plain(id, "subsonic", "text"))
                .await
                .unwrap();
        }

        invalidate_songs(&pool, &["s1".to_string(), "s3".to_string()])
            .await
            .unwrap();
        assert!(load_cached(&pool, "s1").await.unwrap().is_none());
        assert!(load_cached(&pool, "s2").await.unwrap().is_some());
        assert!(load_cached(&pool, "s3").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_removes_lyrics_for_songs_that_left_the_library() {
        let lyrics = lyrics_pool().await;
        let library = library_pool().await;
        sqlx::query("CREATE TABLE songs (id TEXT PRIMARY KEY)")
            .execute(&library)
            .await
            .unwrap();
        sqlx::query("INSERT INTO songs (id) VALUES ('keep')")
            .execute(&library)
            .await
            .unwrap();

        store(&lyrics, &Document::plain("keep", "legacy", "a"))
            .await
            .unwrap();
        store(&lyrics, &Document::plain("gone", "legacy", "b"))
            .await
            .unwrap();

        let removed = purge_with(&lyrics, &library).await.unwrap();
        assert_eq!(removed, 1);
        assert!(load_cached(&lyrics, "keep").await.unwrap().is_some());
        assert!(load_cached(&lyrics, "gone").await.unwrap().is_none());
    }

    #[test]
    fn backfill_candidates_skip_hits_and_fresh_misses() {
        let now = 1_000_000;
        let songs = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let rows = vec![
            ("a".to_string(), "subsonic".to_string(), now - 10),
            ("b".to_string(), "none".to_string(), now - 10),
            (
                "c".to_string(),
                "none".to_string(),
                now - MISS_TTL_SECS - 10,
            ),
        ];

        // a = hit, b = fresh miss (skip); c = expired miss, d = unknown.
        assert_eq!(
            candidates(&songs, &rows, now, 10),
            vec!["c".to_string(), "d".to_string()]
        );
        // The cap bounds one sync pass.
        assert_eq!(candidates(&songs, &rows, now, 1).len(), 1);
    }
}
