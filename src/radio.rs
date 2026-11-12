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

//! Similar-song radio (ARC-RS-1): one seed row, expanded through
//! `getSimilarSongs` into the RadioEntry page and its playback queue.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::library::SongRow;

/// The page rows are the queue order. Caching them keeps a click from
/// re-running `getSimilarSongs` and playing a different random row.
static RADIO_ROWS: LazyLock<Mutex<HashMap<String, Vec<SongRow>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Serializes the first expansion for a seed. Opening the page and starting
/// its queue can race, and two `getSimilarSongs` calls may return different
/// random lists; the first caller owns the cached order.
static RADIO_FETCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn cache_key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

fn remember_rows(kind: &str, id: &str, rows: &[SongRow]) {
    let mut cache = RADIO_ROWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.insert(cache_key(kind, id), rows.to_vec());
}

fn cached_rows(kind: &str, id: &str) -> Option<Vec<SongRow>> {
    RADIO_ROWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_key(kind, id))
        .cloned()
}

fn song_rows(children: &[crate::api::Child]) -> Vec<SongRow> {
    children
        .iter()
        .enumerate()
        .map(|(index, child)| SongRow {
            id: child.id.clone(),
            title: child.title.clone(),
            artist: child.artist.clone(),
            album: child.album.clone(),
            album_id: child.album_id.clone(),
            track: Some(index as i32 + 1),
            duration: child.duration,
            suffix: child.suffix.clone(),
            cover_art: child.cover_art.clone(),
            starred: child.starred.clone(),
        })
        .collect()
}

/// Result of the Tool > Save Radio action. Local save is authoritative; the
/// remote playlist is best-effort and reports its failure without losing the
/// local copy.
#[derive(Debug, Clone)]
pub struct SavedRadio {
    pub name: String,
    pub path: std::path::PathBuf,
    pub remote_error: Option<String>,
}

/// The generated rows for a radio seed: the cached page order when available,
/// otherwise a fresh `getSimilarSongs` expansion recorded for playback.
pub async fn radio_rows(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
) -> Result<Vec<SongRow>> {
    if let Some(rows) = cached_rows(seed_kind, seed_id) {
        return Ok(rows);
    }
    let _guard = RADIO_FETCH.lock().await;
    if let Some(rows) = cached_rows(seed_kind, seed_id) {
        return Ok(rows);
    }
    let seed = resolve_seed(dbs, seed_kind, seed_id).await?;
    let children = similar_songs(dbs, &seed).await?;
    let mut rows = song_rows(&children);
    let song_ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let album_covers = crate::library::song_cover_ids(&dbs.library, &song_ids).await?;
    for row in &mut rows {
        if let Some(cover) = album_covers.get(&row.id) {
            row.cover_art = Some(cover.clone());
        }
    }
    remember_rows(seed_kind, seed_id, &rows);
    Ok(rows)
}

struct Seed {
    song_id: String,
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
                song_id: first.id.clone(),
                title: album.title,
                subtitle: album.artist.unwrap_or_default(),
                cover_art: album.cover_art.or(first.cover_art),
            })
        }
        // A song row can be a server-only track (radio or playlist rows), so
        // the local lookup decorates the seed when possible and otherwise
        // still seeds the API by id.
        "song" => {
            if let Some(song) = crate::library::song(&dbs.library, id).await? {
                return Ok(Seed {
                    song_id: song.id,
                    title: song.title,
                    subtitle: song.artist.unwrap_or_default(),
                    cover_art: song.cover_art,
                });
            }
            Ok(Seed {
                song_id: id.to_string(),
                title: "Radio".to_string(),
                subtitle: String::new(),
                cover_art: None,
            })
        }
        other => Err(anyhow!("unknown radio seed kind: {other}")),
    }
}

async fn similar_songs(dbs: &crate::db::Dbs, seed: &Seed) -> Result<Vec<crate::api::Child>> {
    let client = crate::session::client(dbs).await?;
    client.get_similar_songs(&seed.song_id, Some(100)).await
}

/// The RadioEntry payload: seed identity, similar-song rows and a cover
/// collage. Offline still returns the seed identity so the page can say why
/// it is empty.
pub async fn page(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    offline: bool,
) -> Result<(crate::model::PageHeader, Vec<crate::model::Row>)> {
    use crate::model::{PageHeader, Row, RowKind};
    let seed = resolve_seed(dbs, seed_kind, seed_id).await?;
    let subtitle = if offline {
        "Radio needs a connection".to_string()
    } else if seed.subtitle.is_empty() {
        "Radio".to_string()
    } else {
        format!("Radio · {}", seed.subtitle)
    };
    let header = |collage: Vec<String>, collage_remote: Vec<String>, collage_art: Vec<String>| {
        PageHeader {
            title: seed.title.clone(),
            subtitle: subtitle.clone(),
            meta: "Similar songs".to_string(),
            cover_url: collage.first().cloned().unwrap_or_default(),
            cover_remote: collage_remote.first().cloned().unwrap_or_default(),
            collage,
            collage_remote,
            collage_art,
            cover_art: String::new(),
            rating: 0,
            starred: false,
        }
    };
    if offline {
        return Ok((header(Vec::new(), Vec::new(), Vec::new()), Vec::new()));
    }

    // The page and the playback queue must share one expansion. `radio_rows`
    // caches under a fetch lock; use it here instead of fetching again.
    let rows = radio_rows(dbs, seed_kind, seed_id).await?;
    let cover_ids = std::iter::once(&seed.cover_art).chain(rows.iter().map(|row| &row.cover_art));
    let covers = crate::covers::url_map_for(dbs, cover_ids.clone()).await?;
    let remotes = crate::covers::remote_map_for(dbs, cover_ids).await?;

    let mut collage: Vec<String> = Vec::new();
    let mut collage_remote: Vec<String> = Vec::new();
    let mut collage_art: Vec<String> = Vec::new();
    let seed_cover = crate::covers::url_for(seed.cover_art.as_deref(), &covers);
    if !seed_cover.is_empty() {
        let id = seed.cover_art.clone().unwrap_or_default();
        collage.push(seed_cover);
        collage_remote.push(
            seed.cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default(),
        );
        collage_art.push(id);
    }
    for row in &rows {
        let cover = crate::covers::url_for_ref(row.cover_art.as_deref(), &covers);
        if !cover.is_empty() && !collage.iter().any(|url| url == cover) {
            collage.push(cover.to_string());
            collage_remote.push(
                row.cover_art
                    .as_deref()
                    .and_then(|id| remotes.get(id))
                    .cloned()
                    .unwrap_or_default(),
            );
            collage_art.push(row.cover_art.clone().unwrap_or_default());
        }
    }

    let mut page_rows: Vec<Row> = Vec::with_capacity(rows.len());
    for row in &rows {
        page_rows.push(Row {
            track: String::new(),
            id: row.id.clone(),
            kind: RowKind::Song,
            title: row.title.clone(),
            subtitle: row.artist.clone().or_else(|| row.album.clone()).unwrap_or_default(),
            meta: crate::browse::rows::format_duration(row.duration),
            cover_url: crate::covers::url_for_ref(row.cover_art.as_deref(), &covers).to_string(),
            cover_remote: row
                .cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default(),
            song_id: row.id.clone(),
            album_id: row.album_id.clone().unwrap_or_default(),
            starred: row.starred.is_some(),
            rating: 0,
        });
    }

    Ok((header(collage, collage_remote, collage_art), page_rows))
}

/// The radio queue: the same similar-song list, in server order. The clicked
/// row becomes the queue start.
pub async fn tracks(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    start: i64,
) -> Result<(Vec<crate::player::Track>, i64)> {
    let songs = radio_rows(dbs, seed_kind, seed_id).await?;
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
    let mut songs = radio_rows(dbs, seed_kind, seed_id).await?;
    if songs.is_empty() {
        return Err(anyhow!("this radio has no songs to shuffle"));
    }
    crate::queue::shuffle_songs(&mut songs, crate::consts::now_nanos());
    crate::offline::retain_songs(dbs, offline, &mut songs).await?;
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
    let path = write_radio_m3u(dbs, &dir, &name, &rows).await?;

    let ids: Vec<String> = rows
        .iter()
        .map(|row| row.id.clone())
        .filter(|id| !id.is_empty())
        .collect();
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
    Ok(SavedRadio {
        name,
        path,
        remote_error,
    })
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
    let path = write_radio_m3u(dbs, &dir, &name, &rows).await?;
    Ok(SavedRadio {
        name,
        path,
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
        remember_rows("album", "order-test", &rows);
        let cached = cached_rows("album", "order-test").expect("cached rows");
        assert_eq!(
            cached
                .iter()
                .map(|song| song.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert!(cached_rows("song", "order-test").is_none());
    }
}
