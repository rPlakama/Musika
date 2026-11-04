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
use serde_json::json;

use crate::library::SongRow;

/// The page rows are the queue order. Caching them keeps a click from
/// re-running `getSimilarSongs` and playing a different random row.
static RADIO_ROWS: LazyLock<Mutex<HashMap<String, Vec<SongRow>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
pub async fn page_json(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    offline: bool,
) -> Result<String> {
    let seed = resolve_seed(dbs, seed_kind, seed_id).await?;
    let subtitle = if seed.subtitle.is_empty() {
        "Radio".to_string()
    } else {
        format!("Radio · {}", seed.subtitle)
    };
    let mut page = json!({
        "seed_kind": seed_kind,
        "seed_id": seed_id,
        "title": seed.title,
        "subtitle": subtitle,
        "meta": "Similar songs",
        "song_count": 0,
        "duration": 0,
        "collage": [],
        "songs": [],
    });
    if offline {
        page["subtitle"] = json!("Radio needs a connection");
        return Ok(page.to_string());
    }

    let children = similar_songs(dbs, &seed).await?;
    let rows = song_rows(&children);
    remember_rows(seed_kind, seed_id, &rows);
    let covers = crate::covers::url_map_for(
        dbs,
        std::iter::once(&seed.cover_art).chain(children.iter().map(|child| &child.cover_art)),
    )
    .await?;

    let mut collage: Vec<String> = Vec::new();
    let seed_cover = crate::covers::url_for(seed.cover_art.as_deref(), &covers);
    if !seed_cover.is_empty() {
        collage.push(seed_cover);
    }
    for child in &children {
        let cover = crate::covers::url_for_ref(child.cover_art.as_deref(), &covers);
        if !cover.is_empty() && !collage.iter().any(|url| url == cover) && collage.len() < 3 {
            collage.push(cover.to_string());
        }
    }

    let mut duration = 0i64;
    let songs: Vec<serde_json::Value> = children
        .iter()
        .map(|child| {
            duration += i64::from(child.duration.unwrap_or(0));
            json!({
                "track": "",
                "title": child.title,
                "subtitle": child.artist.clone().or_else(|| child.album.clone()).unwrap_or_default(),
                "duration": crate::browse::rows::format_duration(child.duration),
                "cover": crate::covers::url_for_ref(child.cover_art.as_deref(), &covers),
                "songId": child.id,
                "albumId": child.album_id.clone().unwrap_or_default(),
                "starred": child.starred.is_some(),
                "playable": true,
            })
        })
        .collect();

    page["song_count"] = json!(songs.len());
    page["duration"] = json!(duration);
    page["collage"] = json!(collage);
    page["songs"] = json!(songs);
    Ok(page.to_string())
}

/// The radio queue: the same similar-song list, in server order. The clicked
/// row becomes the queue start.
pub async fn tracks_json(
    dbs: &crate::db::Dbs,
    seed_kind: &str,
    seed_id: &str,
    start: i64,
) -> Result<(String, i64)> {
    let songs = match cached_rows(seed_kind, seed_id) {
        Some(rows) => rows,
        None => {
            let seed = resolve_seed(dbs, seed_kind, seed_id).await?;
            let children = similar_songs(dbs, &seed).await?;
            let rows = song_rows(&children);
            remember_rows(seed_kind, seed_id, &rows);
            rows
        }
    };
    let start = start.clamp(0, songs.len().saturating_sub(1) as i64);
    Ok((crate::queue::tracks_json(dbs, songs).await?, start))
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
