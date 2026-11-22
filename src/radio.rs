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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Similar-song radio (ARC-RS-1): one seed row expanded locally by
//! [`crate::recommend`] into the RadioEntry page and its playback queue.
//!
//! The expansion is offline-first: it never calls the server's
//! `getSimilarSongs`. A cached Navidrome similar-artist graph is preferred
//! when present; metadata similarity fills the rest. Page and queue share the
//! same cached order, and `offline` restricts both to downloaded songs.

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::library::SongRow;

/// How many seed expansions the process keeps in [`RADIO_ROWS`]. N-11: the
/// map used to grow for the whole session; every opened radio would pin its
/// row window. A handful of recent radios is enough to make the page/queue
/// pair (and quick re-opens) reuse one order.
const RADIO_CACHE_CAP: usize = 32;

/// The page rows are the queue order. Caching them keeps a click from
/// re-running the expansion and playing a different random row.
///
/// The cache is LRU-bounded by insertion: a hit moves the seed to the back and
/// the least recently used seed order is evicted past [`RADIO_CACHE_CAP`].
#[derive(Default)]
struct RadioRowsCache {
    rows: HashMap<String, Vec<SongRow>>,
    order: VecDeque<String>,
}

impl RadioRowsCache {
    fn get(&mut self, key: &str) -> Option<Vec<SongRow>> {
        let rows = self.rows.get(key)?.clone();
        self.touch(key);
        Some(rows)
    }

    fn insert(&mut self, key: String, rows: Vec<SongRow>) {
        if self.rows.insert(key.clone(), rows).is_none() {
            self.order.push_back(key);
        } else {
            self.touch(&key);
        }
        while self.order.len() > RADIO_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.rows.remove(&oldest);
            }
        }
    }

    fn touch(&mut self, key: &str) {
        if let Some(position) = self.order.iter().position(|cached| cached == key)
            && let Some(key) = self.order.remove(position)
        {
            self.order.push_back(key);
        }
    }

    fn clear(&mut self) {
        self.rows.clear();
        self.order.clear();
    }
}

static RADIO_ROWS: LazyLock<Mutex<RadioRowsCache>> =
    LazyLock::new(|| Mutex::new(RadioRowsCache::default()));

/// Serializes the first expansion for a seed. Opening the page and starting
/// its queue can race, and two expansions may return different random lists;
/// the first caller owns the cached order.
static RADIO_FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn cache_key(kind: &str, id: &str, offline: bool) -> String {
    // N-08: the offline expansion selects only downloaded candidates, so the
    // two modes are different orders and must not share a cache entry.
    format!("{kind}:{id}:{}", if offline { "offline" } else { "online" })
}

fn remember_rows(kind: &str, id: &str, offline: bool, rows: &[SongRow]) {
    RADIO_ROWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(cache_key(kind, id, offline), rows.to_vec());
}

fn cached_rows(kind: &str, id: &str, offline: bool) -> Option<Vec<SongRow>> {
    RADIO_ROWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_key(kind, id, offline))
}

/// Result of the Tool > Save Radio action. Local save is authoritative; the
/// remote playlist is best-effort and reports its failure without losing the
/// local copy.
#[derive(Debug, Clone)]
pub struct SavedRadio {
    pub name: String,
    pub remote_error: Option<String>,
}

/// The generated rows for a radio seed: the cached page order when available,
/// otherwise a fresh local expansion recorded for playback.
///
/// This is the full (online) mix, used by the save/download actions. The page
/// and queue paths use [`radio_rows_mode`] so an offline expansion restricts
/// candidates *before* selection instead of filtering a full mix afterwards.
pub async fn radio_rows(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
) -> Result<Vec<SongRow>> {
    radio_rows_mode(dbs, seed_kind, seed_id, false).await
}

/// The expansion backing the page and queue. `offline` is passed into
/// [`crate::recommend::similar_songs`] (N-08): candidates are restricted to
/// downloaded songs before the weighted pick, so an offline radio fills its
/// window with playable rows instead of randomly picking server-only ones and
/// filtering them away afterwards.
async fn radio_rows_mode(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    offline: bool,
) -> Result<Vec<SongRow>> {
    if let Some(rows) = cached_rows(seed_kind, seed_id, offline) {
        return Ok(rows);
    }
    let _guard = RADIO_FETCH.lock().await;
    if let Some(rows) = cached_rows(seed_kind, seed_id, offline) {
        return Ok(rows);
    }
    let mut rows = crate::recommend::similar_songs(
        dbs,
        seed_kind,
        seed_id,
        crate::recommend::RADIO_COUNT,
        offline,
        crate::consts::now_nanos() as u64,
    )
    .await?;
    let song_ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let album_covers = crate::library::song_cover_ids(&dbs.library, &song_ids).await?;
    for row in &mut rows {
        if let Some(cover) = album_covers.get(&row.id) {
            row.cover_art = Some(cover.clone());
        }
    }
    remember_rows(seed_kind, seed_id, offline, &rows);
    Ok(rows)
}

/// Drops every cached radio order. Called after a library sync so a removed or
/// re-tagged song cannot linger in a cached expansion.
pub(crate) fn clear_cache() {
    RADIO_ROWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

struct Seed {
    title: String,
    subtitle: String,
    cover_art: Option<String>,
}

/// The seed is an album row (the Shift-Alt arm's row) or a song row. The API
/// seeds from a song id, so an album resolves to its first track.
async fn resolve_seed(dbs: &crate::db::Dbs, kind: &str, id: &str) -> Result<Seed> {
    match kind {
        "album" => {
            let album = crate::library::album(&dbs.library, id)
                .await?
                .ok_or_else(|| anyhow!("radio seed album not found"))?;
            let first = crate::library::album_songs(&dbs.library, id)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("radio seed album has no songs"))?;
            Ok(Seed {
                title: album.title,
                subtitle: album.artist.unwrap_or_default(),
                cover_art: album.cover_art.or(first.cover_art),
            })
        }
        // Local radio needs local metadata. A server-only row that never made
        // it into the synced library cannot be expanded offline; say so
        // instead of silently falling back to the network.
        "song" => {
            let song = crate::library::song(&dbs.library, id)
                .await?
                .ok_or_else(|| anyhow!("this song is not in the local library"))?;
            Ok(Seed {
                title: song.title,
                subtitle: song.artist.unwrap_or_default(),
                cover_art: song.cover_art,
            })
        }
        other => Err(anyhow!("unknown radio seed kind: {other}")),
    }
}

/// The RadioEntry payload: seed identity, similar-song rows and a cover
/// collage. Offline radio lists only downloaded songs; the seed identity stays
/// visible even when that subset is empty.
pub async fn page(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    offline: bool,
) -> Result<(crate::model::PageHeader, Vec<crate::model::Row>)> {
    use crate::model::{PageHeader, Row, RowKind};
    let seed = resolve_seed(dbs, seed_kind, seed_id).await?;
    let subtitle = if offline {
        if seed.subtitle.is_empty() {
            "Radio (offline)".to_string()
        } else {
            format!("Radio (offline) · {}", seed.subtitle)
        }
    } else if seed.subtitle.is_empty() {
        "Radio".to_string()
    } else {
        format!("Radio · {}", seed.subtitle)
    };
    let header =
        |collage: Vec<String>, collage_remote: Vec<String>, collage_art: Vec<String>| PageHeader {
            title: seed.title.clone(),
            subtitle: subtitle.clone(),
            meta: "Similar songs · local".to_string(),
            cover_url: collage.first().cloned().unwrap_or_default(),
            cover_remote: collage_remote.first().cloned().unwrap_or_default(),
            collage,
            collage_remote,
            collage_art,
            cover_art: String::new(),
            rating: 0,
            starred: false,
        };
    // The page and the playback queue must share one expansion. `radio_rows`
    // caches under a fetch lock; use it here instead of fetching again. The
    // offline flag is part of the expansion (N-08), so the cached order only
    // ever holds playable rows; the final `retain_songs` re-checks the download
    // catalog in case it changed since the expansion was cached.
    let mut rows = radio_rows_mode(dbs, seed_kind, seed_id, offline).await?;
    crate::offline::retain_songs(dbs, offline, &mut rows).await?;
    let cover_ids = std::iter::once(&seed.cover_art).chain(rows.iter().map(|row| &row.cover_art));
    let covers = crate::covers::url_map_for(dbs, cover_ids.clone()).await?;
    let remotes = crate::covers::remote_map_for(dbs, cover_ids).await?;

    // Same deterministic rule as playlist pages: a pane is defined by its
    // cover id, not by whether the local file is already cached. Otherwise the
    // collage changes shape while the cover pass warms up.
    let mut collage: Vec<String> = Vec::new();
    let mut collage_remote: Vec<String> = Vec::new();
    let mut collage_art: Vec<String> = Vec::new();
    let seed_id = seed.cover_art.clone().filter(|id| !id.is_empty());
    if let Some(id) = seed_id {
        collage_art.push(id.clone());
        collage.push(crate::covers::url_for(Some(&id), &covers));
        collage_remote.push(remotes.get(&id).cloned().unwrap_or_default());
    }
    for row in &rows {
        if collage.len() >= 4 {
            break;
        }
        let Some(id) = row.cover_art.clone().filter(|id| !id.is_empty()) else {
            continue;
        };
        if collage_art.contains(&id) {
            continue;
        }
        collage_art.push(id.clone());
        collage.push(crate::covers::url_for(Some(&id), &covers));
        collage_remote.push(remotes.get(&id).cloned().unwrap_or_default());
    }

    let mut page_rows: Vec<Row> = Vec::with_capacity(rows.len());
    for row in &rows {
        page_rows.push(Row {
            item: crate::model::ItemMeta {
                id: row.id.clone(),
                title: row.title.clone(),
                cover_url: crate::covers::url_for_ref(row.cover_art.as_deref(), &covers)
                    .to_string(),
                cover_remote: row
                    .cover_art
                    .as_deref()
                    .and_then(|id| remotes.get(id))
                    .cloned()
                    .unwrap_or_default(),
                album_id: row.album_id.clone().unwrap_or_default(),
            },
            track: String::new(),
            kind: RowKind::Song,
            subtitle: row
                .artist
                .clone()
                .or_else(|| row.album.clone())
                .unwrap_or_default(),
            meta: crate::browse::rows::format_duration(row.duration),
            song_id: row.id.clone(),
            starred: row.starred.is_some(),
            rating: 0,
        });
    }

    Ok((header(collage, collage_remote, collage_art), page_rows))
}

/// The radio queue: the same local mix, in the cached page order. The clicked
/// row becomes the queue start; the queue applies the same offline filter as
/// [`page`], so the page row index maps to the same queue index.
pub async fn tracks(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    start: i64,
    offline: bool,
) -> Result<(Vec<crate::player::Track>, i64)> {
    let mut songs = radio_rows_mode(dbs, seed_kind, seed_id, offline).await?;
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    let start = start.clamp(0, songs.len().saturating_sub(1) as i64);
    Ok((crate::queue::tracks(dbs, songs).await?, start))
}

/// Shuffled radio queue: the page's generated rows, in a fresh Fisher–Yates
/// order. Used by the Radio Tool board's Shuffle action.
pub async fn shuffled_tracks(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    offline: bool,
) -> Result<Vec<crate::player::Track>> {
    let mut songs = radio_rows_mode(dbs, seed_kind, seed_id, offline).await?;
    // N-08: filter *before* the emptiness check, so an offline radio whose
    // cached order has no downloaded rows reports "no songs" instead of
    // returning a silent empty queue.
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
    if songs.is_empty() {
        return Err(anyhow!("this radio has no songs to shuffle"));
    }
    crate::queue::shuffle_songs(&mut songs, crate::consts::now_nanos());
    crate::queue::tracks(dbs, songs).await
}

/// Saves the generated radio twice: an app-owned M3U in the local playlist
/// folder and a server playlist through `createPlaylist`.
///
/// The local file is written and scanned first. A missing client or a failing
/// `createPlaylist` is returned in [`SavedRadio::remote_error`] so the UI can
/// say "saved locally, server failed" without discarding the local copy.
pub async fn save_radio(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    name: &str,
) -> Result<SavedRadio> {
    let rows = radio_rows(dbs, seed_kind, seed_id).await?;
    if rows.is_empty() {
        return Err(anyhow!("this radio has no songs to save"));
    }
    let name = if name.trim().is_empty() {
        "Radio".to_string()
    } else {
        name.trim().to_string()
    };

    let dir = crate::playlists::m3u_dir()?;
    _ = write_radio_m3u(dbs, &dir, &name, &rows).await?;

    // The server playlist carries the same invariant as every other playlist:
    // no duplicate entries, even if the radio expansion ever repeats a song.
    let ids = crate::playlists::dedup_keep_first_ids(
        &rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
    );
    let remote_error = match crate::session::client(dbs).await {
        Ok(client) => match client.create_playlist(&name, &ids).await {
            Ok(()) => {
                // Keep the local list in step with the playlist that just
                // appeared on the server.
                if let Err(err) = crate::playlists::refresh_subsonic(dbs, &client).await {
                    eprintln!("radio: saved playlist list refresh failed: {err:#}");
                }
                None
            }
            Err(err) => Some(format!("{err:#}")),
        },
        Err(err) => Some(format!("{err:#}")),
    };
    Ok(SavedRadio { name, remote_error })
}

/// Save the radio only as an app-owned local M3U (`radio save --offline`).
pub async fn save_radio_local(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    name: &str,
) -> Result<SavedRadio> {
    let rows = radio_rows(dbs, seed_kind, seed_id).await?;
    if rows.is_empty() {
        return Err(anyhow!("this radio has no songs to save"));
    }
    let name = if name.trim().is_empty() {
        "Radio".to_string()
    } else {
        name.trim().to_string()
    };
    let dir = crate::playlists::m3u_dir()?;
    _ = write_radio_m3u(dbs, &dir, &name, &rows).await?;
    Ok(SavedRadio {
        name,
        remote_error: None,
    })
}

/// Writes one saved-radio M3U and refreshes the local playlist index. Split
/// from [`save_radio`] so the file/scan half is testable without a server.
pub(crate) async fn write_radio_m3u(
    dbs: &crate::db::Dbs,
    dir: &std::path::Path,
    name: &str,
    rows: &[SongRow],
) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let paths = song_paths(
        dbs,
        &rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
    )
    .await?;
    let path = dir.join(m3u_filename(dir, name));
    std::fs::write(&path, m3u_text(rows, &paths))?;
    crate::playlists::scan_dir(dbs, dir).await?;
    Ok(path)
}

/// Library paths for the saved M3U. Server-only tracks fall back to the
/// app-owned `musika:song:<id>` entry that [`crate::playlists::Matcher`]
/// resolves back to `song_id` when the track is in the local library.
async fn song_paths(dbs: &crate::db::Dbs, ids: &[String]) -> Result<HashMap<String, String>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut builder = sqlx::QueryBuilder::new("SELECT id, path FROM songs WHERE id IN (");
    crate::cache::push_in_list(&mut builder, ids);
    let rows: Vec<(String, Option<String>)> =
        builder.build_query_as().fetch_all(&dbs.library).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, path)| path.filter(|path| !path.is_empty()).map(|path| (id, path)))
        .collect())
}

fn m3u_line(value: &str) -> String {
    value.replace(['\r', '\n'], " ").trim().to_string()
}

fn m3u_text(rows: &[SongRow], paths: &HashMap<String, String>) -> String {
    let mut text = String::from("#EXTM3U\n");
    for row in rows {
        let artist = row.artist.as_deref().unwrap_or_default();
        let display = if artist.is_empty() {
            row.title.clone()
        } else {
            format!("{artist} - {}", row.title)
        };
        text.push_str(&format!(
            "#EXTINF:{},{}\n",
            row.duration.unwrap_or(0),
            m3u_line(&display)
        ));
        let path = paths
            .get(&row.id)
            .cloned()
            .unwrap_or_else(|| format!("musika:song:{}", row.id));
        text.push_str(&m3u_line(&path));
        text.push('\n');
    }
    text
}

/// A local M3U name that also reads well in the Playlists segment (the m3u
/// scanner uses the file name as the playlist name). The same radio title
/// overwrites its existing mix rather than creating timestamped duplicates.
fn m3u_filename(_dir: &std::path::Path, name: &str) -> String {
    let mut safe = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '(' | ')' | '&' | '\'' | '·') {
            safe.push(ch);
        } else if !safe.ends_with('-') {
            safe.push('-');
        }
    }
    let safe = safe.trim().trim_end_matches('-').trim();
    let safe = if safe.is_empty() { "Radio" } else { safe };
    format!("{safe}.m3u")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str) -> SongRow {
        SongRow {
            id: id.to_string(),
            title: id.to_string(),
            artist: None,
            album: None,
            album_id: None,
            track: None,
            duration: None,
            suffix: None,
            cover_art: None,
            starred: None,
        }
    }

    #[test]
    fn saved_radio_m3u_keeps_order_and_falls_back_to_song_ids() {
        let mut a = row("a");
        a.artist = Some("Artist".into());
        a.duration = Some(42);
        let b = row("b");
        let rows = vec![a, b];
        let paths = HashMap::from([("a".to_string(), "Music/a.flac".to_string())]);
        let text = m3u_text(&rows, &paths);
        assert!(text.starts_with("#EXTM3U\n"));
        assert!(text.contains("#EXTINF:42,Artist - a\nMusic/a.flac\n"));
        assert!(text.contains("musika:song:b\n"));
        assert_eq!(
            m3u_filename(
                std::path::Path::new("/nonexistent"),
                "A Beginner's Mind · Radio"
            ),
            "A Beginner's Mind · Radio.m3u"
        );
    }

    #[test]
    fn cached_page_rows_keep_the_displayed_order() {
        let rows = vec![row("b"), row("a"), row("c")];
        let mut cache = RadioRowsCache::default();
        cache.insert(cache_key("album", "order-test", false), rows);
        let cached = cache
            .get(&cache_key("album", "order-test", false))
            .expect("cached rows");
        assert_eq!(
            cached
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert!(cache.get(&cache_key("song", "order-test", false)).is_none());
        assert!(
            cache.get(&cache_key("album", "order-test", true)).is_none(),
            "offline and online expansions are separate cache entries"
        );
    }

    /// N-11: the cache must never grow past its cap.
    #[test]
    fn row_cache_is_bounded_and_evicts_oldest_seeds() {
        let mut cache = RadioRowsCache::default();
        for seed in 0..(RADIO_CACHE_CAP + 8) {
            cache.insert(
                cache_key("album", &format!("seed-{seed}"), false),
                vec![row("a")],
            );
        }
        assert_eq!(cache.rows.len(), RADIO_CACHE_CAP);
        assert_eq!(cache.order.len(), RADIO_CACHE_CAP);
        assert!(
            cache
                .rows
                .contains_key(&cache_key("album", "seed-8", false))
        );
        assert!(
            !cache
                .rows
                .contains_key(&cache_key("album", "seed-0", false)),
            "the oldest seed order is evicted first"
        );
    }

    /// A cache hit refreshes the entry's recency so hot radios survive.
    #[test]
    fn row_cache_hit_refreshes_recency() {
        let mut cache = RadioRowsCache::default();
        cache.insert(cache_key("album", "hot", false), vec![row("a")]);
        for seed in 0..(RADIO_CACHE_CAP - 1) {
            cache.insert(
                cache_key("album", &format!("cold-{seed}"), false),
                vec![row("b")],
            );
        }
        let _ = cache.get(&cache_key("album", "hot", false));
        cache.insert(cache_key("album", "newest", false), vec![row("c")]);
        assert!(
            cache.rows.contains_key(&cache_key("album", "hot", false)),
            "a recently read seed is not the eviction victim"
        );
        assert!(
            !cache
                .rows
                .contains_key(&cache_key("album", "cold-0", false)),
            "the least recently used seed is evicted"
        );
    }
}
