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
//!
//! N-08: the filtered readers are bounded. The count is a `COUNT(*)` over the
//! downloaded ids (chunked by [`crate::cache::IN_BATCH`]) instead of loading
//! the whole library and filtering it in RAM, and the window scan reads only
//! as many rows as it needs to reach the requested page. Filtered album/artist
//! rows also report the *downloaded* track/album counts, not the full server
//! catalog counts.

use std::collections::{HashMap, HashSet};

use anyhow::bail;

use crate::db::Dbs;
use crate::model::Row;

/// Rows read per batch while streaming a filtered window. Bounds the filtered
/// reader's memory to this many rows instead of the whole catalog.
const FILTER_SCAN_BATCH: i64 = 512;

/// The downloaded catalog in id-set form, for filtering without loading the
/// whole library (N-08).
///
/// [`crate::offline::load`] exposes membership checks only; the count/window
/// readers need the ids so SQLite can apply the filter and so the window scan
/// stops after the requested page. The map values are the downloaded
/// track/album counts the row metadata displays.
struct DownloadedCatalog {
    songs: HashSet<String>,
    /// album id -> number of downloaded tracks.
    albums: HashMap<String, i64>,
    /// artist id -> number of albums with at least one downloaded track.
    artists: HashMap<String, i64>,
}

/// The downloaded catalog when this surface must be filtered (`offline`, or
/// the downloaded ordering); `None` means the full library.
async fn surface_filter(
    dbs: &Dbs,
    order: &str,
    offline: bool,
) -> anyhow::Result<Option<DownloadedCatalog>> {
    if !offline && !crate::library::is_downloaded_order(order) {
        return Ok(None);
    }
    // The ledger is the source of truth for the offline catalog; the same two
    // queries [`crate::offline::load`] runs, because only the ids (not just
    // membership) let the rows be counted and windowed in SQL.
    let songs = crate::download::downloaded_song_ids(dbs).await?;
    let ids: Vec<String> = songs.iter().cloned().collect();
    let (albums, artists) = crate::library::downloaded_owners(&dbs.library, &ids).await?;
    Ok(Some(DownloadedCatalog {
        songs,
        albums,
        artists,
    }))
}

/// Rows shared by the three filtered window readers: read one batch at a time,
/// keep the matches and stop once the requested page is filled. The loader
/// returns an owned future (it clones the pool handle per batch) so the helper
/// stays a plain generic over the batch readers.
async fn scan_filtered<T, F, Fut, K>(
    start: usize,
    count: usize,
    mut load: F,
    mut keep: K,
) -> anyhow::Result<Vec<T>>
where
    F: FnMut(i64, i64) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<T>>>,
    K: FnMut(&T) -> bool,
{
    let mut window = Vec::with_capacity(count);
    let mut matched = 0usize;
    let mut offset = 0i64;
    loop {
        let batch = load(offset, FILTER_SCAN_BATCH).await?;
        let fetched = batch.len() as i64;
        for row in batch {
            if !keep(&row) {
                continue;
            }
            if matched >= start {
                window.push(row);
                if window.len() >= count {
                    return Ok(window);
                }
            }
            matched += 1;
        }
        if fetched < FILTER_SCAN_BATCH {
            return Ok(window);
        }
        offset += fetched;
    }
}

/// Counts the library rows whose id is in one downloaded-id set, chunked by
/// [`crate::cache::IN_BATCH`] so the count never materializes a full table.
async fn count_filtered(
    pool: &sqlx::sqlite::SqlitePool,
    select: &'static str,
    ids: &[&str],
    tail: &'static str,
    bind: Option<&str>,
) -> anyhow::Result<usize> {
    let mut total = 0usize;
    for chunk in ids.chunks(crate::cache::IN_BATCH) {
        let mut builder = sqlx::QueryBuilder::new(select);
        crate::cache::push_in_list(&mut builder, chunk);
        builder.push(tail);
        let mut query = builder.build_query_scalar::<i64>();
        if let Some(value) = bind {
            query = query.bind(value);
        }
        let count: i64 = query.fetch_one(pool).await?;
        total += count.max(0) as usize;
    }
    Ok(total)
}

pub(crate) async fn count(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    order: &str,
    offline: bool,
) -> anyhow::Result<usize> {
    if let Some(catalog) = surface_filter(dbs, order, offline).await? {
        // Count exactly what the window reader can show: the filtered library
        // lists (a dangling download id must not make the virtual list believe
        // in a row that has no metadata). N-08: SQL does the filtering, so the
        // per-count RAM stays bounded by the id list, not the full catalog.
        return match surface {
            "albums" => {
                let artist = (!key.is_empty()).then_some(key);
                let ids: Vec<&str> = catalog.albums.keys().map(String::as_str).collect();
                let tail = if artist.is_some() {
                    " AND artist = ?"
                } else {
                    ""
                };
                count_filtered(
                    &dbs.library,
                    "SELECT COUNT(*) FROM albums WHERE id IN (",
                    &ids,
                    tail,
                    artist,
                )
                .await
            }
            "artists" => {
                let ids: Vec<&str> = catalog.artists.keys().map(String::as_str).collect();
                count_filtered(
                    &dbs.library,
                    "SELECT COUNT(*) FROM artists WHERE id IN (",
                    &ids,
                    " AND EXISTS (SELECT 1 FROM albums WHERE albums.artist_id = artists.id)",
                    None,
                )
                .await
            }
            "songs" => {
                let ids: Vec<&str> = catalog.songs.iter().map(String::as_str).collect();
                count_filtered(
                    &dbs.library,
                    "SELECT COUNT(*) FROM songs WHERE id IN (",
                    &ids,
                    "",
                    None,
                )
                .await
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
                Some(catalog) => {
                    scan_filtered(
                        start,
                        count,
                        |offset, limit| {
                            let pool = dbs.library.clone();
                            let artist = artist.map(str::to_string);
                            let order = order.to_string();
                            async move {
                                crate::library::albums_range(
                                    &pool,
                                    artist.as_deref(),
                                    &order,
                                    offset,
                                    limit,
                                )
                                .await
                            }
                        },
                        |album| catalog.albums.contains_key(&album.id),
                    )
                    .await?
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
            album_rows(dbs, albums, filter.as_ref().map(|catalog| &catalog.albums)).await
        }
        "artists" => {
            let artists = match &filter {
                Some(catalog) => {
                    scan_filtered(
                        start,
                        count,
                        |offset, limit| {
                            let pool = dbs.library.clone();
                            let order = order.to_string();
                            async move {
                                crate::library::artists_range(&pool, &order, offset, limit).await
                            }
                        },
                        |artist| catalog.artists.contains_key(&artist.id),
                    )
                    .await?
                }
                None => {
                    crate::library::artists_range(&dbs.library, order, start as i64, count as i64)
                        .await?
                }
            };
            artist_rows(
                dbs,
                artists,
                filter.as_ref().map(|catalog| &catalog.artists),
            )
            .await
        }
        "songs" => {
            let songs = match &filter {
                Some(catalog) => {
                    scan_filtered(
                        start,
                        count,
                        |offset, limit| {
                            let pool = dbs.library.clone();
                            let order = order.to_string();
                            async move {
                                crate::library::songs_range(&pool, offset, limit, &order).await
                            }
                        },
                        |song| catalog.songs.contains(&song.id),
                    )
                    .await?
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

async fn album_rows(
    dbs: &Dbs,
    albums: Vec<crate::library::AlbumRow>,
    downloaded_counts: Option<&HashMap<String, i64>>,
) -> anyhow::Result<Vec<Row>> {
    use crate::model::RowKind;
    let covers =
        crate::covers::url_map_for(dbs, albums.iter().map(|album| &album.cover_art)).await?;
    let remotes =
        crate::covers::remote_map_for(dbs, albums.iter().map(|album| &album.cover_art)).await?;
    Ok(albums
        .into_iter()
        .map(|album| {
            // N-08: in the downloaded ordering the count describes what is
            // actually downloadable, not the server's full album size.
            let track_count = downloaded_counts
                .and_then(|counts| counts.get(&album.id).copied())
                .unwrap_or(album.song_count);
            Row {
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
                meta: format!("{track_count} tracks"),
                song_id: String::new(),
                starred: album.starred.is_some(),
                rating: album.rating,
            }
        })
        .collect())
}

async fn artist_rows(
    dbs: &Dbs,
    artists: Vec<crate::library::ArtistRow>,
    downloaded_counts: Option<&HashMap<String, i64>>,
) -> anyhow::Result<Vec<Row>> {
    use crate::model::RowKind;
    let covers =
        crate::covers::url_map_for(dbs, artists.iter().map(|artist| &artist.cover_art)).await?;
    let remotes =
        crate::covers::remote_map_for(dbs, artists.iter().map(|artist| &artist.cover_art)).await?;
    // N-20: the overlay state is read for just this page's artists; the old
    // code loaded the whole `artist_user` table for every window.
    let artist_ids: Vec<&str> = artists.iter().map(|artist| artist.id.as_str()).collect();
    let states = crate::artist_user::states_for(&dbs.artist_user, &artist_ids).await?;
    Ok(artists
        .into_iter()
        .map(|artist| {
            let (starred, rating) = states.get(&artist.id).copied().unwrap_or((false, 0));
            // N-08: report the downloaded album count while filtering.
            let albums = downloaded_counts
                .and_then(|counts| counts.get(&artist.id).copied())
                .unwrap_or(artist.album_count);
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
                starred: artist.starred.is_some() || starred,
                rating,
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
