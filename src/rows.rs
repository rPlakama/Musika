// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Rust-owned row windows for the GPUI virtual list surfaces.
//!
//! Nothing in this module retains a row after it has been emitted: the GPUI
//! side keeps only the live viewport slots, and a new window replaces them.

use anyhow::bail;

use crate::db::Dbs;

pub(crate) fn next_generation(
    generations: &mut std::collections::HashMap<String, u64>,
    surface: &str,
) -> u64 {
    let generation = generations.entry(surface.to_string()).or_insert(0);
    *generation += 1;
    *generation
}

pub(crate) async fn count(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    _order: &str,
    offline: bool,
) -> anyhow::Result<usize> {
    let count = match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            crate::library::album_count(&dbs.library, artist).await?
        }
        "artists" => crate::library::artist_count(&dbs.library).await?,
        "songs" => crate::library::song_count(&dbs.library).await?,
        other => bail!("unknown row surface: {other}"),
    };
    // Offline: the window reader below retains too, so the count must match
    // the filtered set or the virtual list overruns.
    if offline && let Some(filter) = crate::offline::load(dbs, true).await? {
        let windowed = match surface {
            "albums" => {
                let artist = (!key.is_empty()).then_some(key);
                let albums = crate::library::albums(&dbs.library, artist, _order).await?;
                albums
                    .iter()
                    .filter(|a| filter.contains_album(&a.id))
                    .count()
            }
            "artists" => {
                let artists = crate::library::artists(&dbs.library, _order).await?;
                artists
                    .iter()
                    .filter(|a| filter.contains_artist(&a.id))
                    .count()
            }
            _ => {
                let songs = crate::library::all_songs(&dbs.library, _order).await?;
                songs.iter().filter(|s| filter.contains_song(&s.id)).count()
            }
        };
        return Ok(windowed.min(count));
    }
    Ok(count)
}

pub(crate) async fn window(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    order: &str,
    start: usize,
    count: usize,
    offline: bool,
) -> anyhow::Result<Vec<crate::model::Row>> {
    use crate::model::{Row, RowKind};
    let start = start as i64;
    let count = count.max(1) as i64;
    // ARC-RS-4.11: offline filters the window in-memory instead of ignoring
    // it — the full-scope fallback used to materialise the whole list.
    let offline_filter = if offline {
        crate::offline::load(dbs, true).await?
    } else {
        None
    };
    let rows: Vec<Row> = match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            let mut albums =
                crate::library::albums_range(&dbs.library, artist, order, start, count).await?;
            if let Some(filter) = &offline_filter {
                albums.retain(|album| filter.contains_album(&album.id));
            }
            let covers =
                crate::covers::url_map_for(dbs, albums.iter().map(|album| &album.cover_art))
                    .await?;
            let remotes =
                crate::covers::remote_map_for(dbs, albums.iter().map(|album| &album.cover_art))
                    .await?;
            albums
                .into_iter()
                .map(|album| Row {
                    item: crate::model::ItemMeta {
                        id: album.id.clone(),
                        title: album.title.clone(),
                        cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
                        cover_remote: album
                            .cover_art
                            .as_deref()
                            .and_then(|id| remotes.get(id))
                            .cloned()
                            .unwrap_or_default(),
                        album_id: album.id.clone(),
                    },
                    track: String::new(),
                    kind: RowKind::Album,
                    subtitle: crate::browse::rows::album_subtitle(&album),
                    meta: format!("{} tracks", album.song_count),
                    song_id: String::new(),
                    starred: album.starred.is_some(),
                    rating: album.rating,
                })
                .collect()
        }
        "artists" => {
            let mut artists =
                crate::library::artists_range(&dbs.library, order, start, count).await?;
            if let Some(filter) = &offline_filter {
                artists.retain(|artist| filter.contains_artist(&artist.id));
            }
            let covers =
                crate::covers::url_map_for(dbs, artists.iter().map(|artist| &artist.cover_art))
                    .await?;
            let remotes =
                crate::covers::remote_map_for(dbs, artists.iter().map(|artist| &artist.cover_art))
                    .await?;
            let starred: std::collections::HashSet<String> =
                crate::artist_user::starred_ids(&dbs.artist_user)
                    .await?
                    .into_iter()
                    .collect();
            let ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
            artists
                .into_iter()
                .map(|artist| {
                    let albums = artist.album_count;
                    Row {
                        item: crate::model::ItemMeta {
                            id: artist.id.clone(),
                            title: artist.name.clone(),
                            cover_url: crate::covers::url_for(artist.cover_art.as_deref(), &covers),
                            cover_remote: artist
                                .cover_art
                                .as_deref()
                                .and_then(|id| remotes.get(id))
                                .cloned()
                                .unwrap_or_default(),
                            album_id: String::new(),
                        },
                        track: String::new(),
                        kind: RowKind::Artist,
                        subtitle: String::new(),
                        meta: format!("{albums} album{}", if albums == 1 { "" } else { "s" }),
                        song_id: String::new(),
                        starred: artist.starred.is_some() || starred.contains(&artist.id),
                        rating: ratings.get(&artist.id).copied().unwrap_or(0),
                    }
                })
                .collect()
        }
        "songs" => {
            let mut songs = crate::library::songs_range(&dbs.library, start, count, order).await?;
            if let Some(filter) = &offline_filter {
                songs.retain(|song| filter.contains_song(&song.id));
            }
            let covers =
                crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
            let remotes =
                crate::covers::remote_map_for(dbs, songs.iter().map(|song| &song.cover_art))
                    .await?;
            songs
                .into_iter()
                .map(|song| Row {
                    item: crate::model::ItemMeta {
                        id: song.id.clone(),
                        title: song.title.clone(),
                        cover_url: crate::covers::url_for(song.cover_art.as_deref(), &covers),
                        cover_remote: song
                            .cover_art
                            .as_deref()
                            .and_then(|id| remotes.get(id))
                            .cloned()
                            .unwrap_or_default(),
                        album_id: song.album_id.unwrap_or_default(),
                    },
                    track: String::new(),
                    kind: RowKind::Song,
                    subtitle: crate::browse::rows::song_subtitle_from_parts(
                        song.artist.as_deref(),
                        song.album.as_deref(),
                    ),
                    meta: crate::browse::rows::format_duration(song.duration),
                    song_id: song.id,
                    starred: song.starred.is_some(),
                    rating: 0,
                })
                .collect()
        }
        other => bail!("unknown row surface: {other}"),
    };
    Ok(rows)
}
