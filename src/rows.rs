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
//!
//! Offline mode and the [`crate::library::DOWNLOADED_ORDER`] filter both use
//! the downloaded catalog. They select rows *before* the window is sliced:
//! filtering an already-paged window would drop matches and leave holes the
//! virtual list cannot fill.

use anyhow::bail;

use crate::db::Dbs;
use crate::model::Row;

pub(crate) fn next_generation(
    generations: &mut std::collections::HashMap<String, u64>,
    surface: &str,
) -> u64 {
    let generation = generations.entry(surface.to_string()).or_insert(0);
    *generation += 1;
    *generation
}

/// The downloaded catalog when this surface must be filtered (`offline`, or
/// the downloaded ordering); `None` means the full library.
async fn surface_filter(
    dbs: &Dbs,
    order: &str,
    offline: bool,
) -> anyhow::Result<Option<std::sync::Arc<crate::offline::Filter>>> {
    if offline || crate::library::is_downloaded_order(order) {
        // `load(dbs, true)` is the catalog accessor: the `true` is the
        // "downloaded-only" switch, not the runtime connectivity state.
        return crate::offline::load(dbs, true).await;
    }
    Ok(None)
}

fn take_window<T>(items: Vec<T>, start: usize, count: usize) -> Vec<T> {
    items.into_iter().skip(start).take(count).collect()
}

pub(crate) async fn count(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<usize> {
    if let Some(filter) = surface_filter(dbs, order, offline).await? {
        // Count exactly what the window reader can show: the filtered library
        // lists (a dangling download id must not make the virtual list
        // believe in a row that has no metadata).
        return match surface {
            "albums" => {
                let artist = (!key.is_empty()).then_some(key);
                let albums = crate::library::albums(&dbs.library, artist, order).await?;
                Ok(albums
                    .iter()
                    .filter(|album| filter.contains_album(&album.id))
                    .count())
            }
            "artists" => {
                let artists = crate::library::artists(&dbs.library, order).await?;
                Ok(artists
                    .iter()
                    .filter(|artist| filter.contains_artist(&artist.id))
                    .count())
            }
            "songs" => {
                let songs = crate::library::all_songs(&dbs.library, order).await?;
                Ok(songs
                    .iter()
                    .filter(|song| filter.contains_song(&song.id))
                    .count())
            }
            other => bail!("unknown row surface: {other}"),
        };
    }
    match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            Ok(crate::library::album_count(&dbs.library, artist).await?)
        }
        "artists" => Ok(crate::library::artist_count(&dbs.library).await?),
        "songs" => Ok(crate::library::song_count(&dbs.library).await?),
        other => bail!("unknown row surface: {other}"),
    }
}

pub(crate) async fn window(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    order: &str,
    start: usize,
    count: usize,
    offline: bool,
) -> anyhow::Result<Vec<Row>> {
    let count = count.max(1);
    let filter = surface_filter(dbs, order, offline).await?;
    match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            let albums = match &filter {
                Some(filter) => {
                    let mut albums = crate::library::albums(&dbs.library, artist, order).await?;
                    albums.retain(|album| filter.contains_album(&album.id));
                    take_window(albums, start, count)
                }
                None => {
                    crate::library::albums_range(
                        &dbs.library,
                        artist,
                        order,
                        start as i64,
                        count as i64,
                    )
                    .await?
                }
            };
            album_rows(dbs, albums).await
        }
        "artists" => {
            let artists = match &filter {
                Some(filter) => {
                    let mut artists = crate::library::artists(&dbs.library, order).await?;
                    artists.retain(|artist| filter.contains_artist(&artist.id));
                    take_window(artists, start, count)
                }
                None => {
                    crate::library::artists_range(&dbs.library, order, start as i64, count as i64)
                        .await?
                }
            };
            artist_rows(dbs, artists).await
        }
        "songs" => {
            let songs = match &filter {
                Some(filter) => {
                    let mut songs = crate::library::all_songs(&dbs.library, order).await?;
                    songs.retain(|song| filter.contains_song(&song.id));
                    take_window(songs, start, count)
                }
                None => {
                    crate::library::songs_range(&dbs.library, start as i64, count as i64, order)
                        .await?
                }
            };
            song_rows(dbs, songs).await
        }
        other => bail!("unknown row surface: {other}"),
    }
}

async fn album_rows(dbs: &Dbs, albums: Vec<crate::library::AlbumRow>) -> anyhow::Result<Vec<Row>> {
    use crate::model::RowKind;
    let covers =
        crate::covers::url_map_for(dbs, albums.iter().map(|album| &album.cover_art)).await?;
    let remotes =
        crate::covers::remote_map_for(dbs, albums.iter().map(|album| &album.cover_art)).await?;
    Ok(albums
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
        .collect())
}

async fn artist_rows(
    dbs: &Dbs,
    artists: Vec<crate::library::ArtistRow>,
) -> anyhow::Result<Vec<Row>> {
    use crate::model::RowKind;
    let covers =
        crate::covers::url_map_for(dbs, artists.iter().map(|artist| &artist.cover_art)).await?;
    let remotes =
        crate::covers::remote_map_for(dbs, artists.iter().map(|artist| &artist.cover_art)).await?;
    let starred: std::collections::HashSet<String> =
        crate::artist_user::starred_ids(&dbs.artist_user)
            .await?
            .into_iter()
            .collect();
    let ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
    Ok(artists
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
        .collect())
}

async fn song_rows(dbs: &Dbs, songs: Vec<crate::library::SongRow>) -> anyhow::Result<Vec<Row>> {
    use crate::model::RowKind;
    let covers = crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    let remotes =
        crate::covers::remote_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
    Ok(songs
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
        .collect())
}
