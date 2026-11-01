// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! Rust-owned row windows for the virtual list surfaces.
//!
//! Nothing in this module retains a row after it has been emitted: the QML
//! side keeps only the live viewport slots, and a new window replaces them.

use anyhow::bail;
use serde_json::{Value, json};

use crate::db::Dbs;

pub(crate) fn next_generation(generations: &mut std::collections::HashMap<String, u64>, surface: &str) -> u64 {
    let generation = generations.entry(surface.to_string()).or_insert(0);
    *generation += 1;
    *generation
}

pub(crate) async fn count(
    dbs: &Dbs,
    surface: &str,
    key: &str,
    _order: &str,
    _offline: bool,
) -> anyhow::Result<usize> {
    match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            crate::library::album_count(&dbs.library, artist).await
        }
        "artists" => crate::library::artist_count(&dbs.library).await,
        "songs" => crate::library::song_count(&dbs.library).await,
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
) -> anyhow::Result<String> {
    let start = start as i64;
    let count = count.max(1) as i64;
    let rows: Vec<Value> = match surface {
        "albums" => {
            let artist = (!key.is_empty()).then_some(key);
            let albums =
                crate::library::albums_range(&dbs.library, artist, order, start, count).await?;
            let covers = crate::covers::url_map_for(
                dbs,
                albums.iter().map(|album| &album.cover_art),
            )
            .await?;
            albums
                .iter()
                .map(|album| crate::browse::rows::album_json(album, &covers, None))
                .collect()
        }
        "artists" => {
            let artists =
                crate::library::artists_range(&dbs.library, order, start, count).await?;
            let covers = crate::covers::url_map_for(
                dbs,
                artists.iter().map(|artist| &artist.cover_art),
            )
            .await?;
            let starred: std::collections::HashSet<String> =
                crate::artist_user::starred_ids(&dbs.artist_user)
                    .await?
                    .into_iter()
                    .collect();
            let ratings = crate::artist_user::ratings(&dbs.artist_user).await?;
            artists
                .iter()
                .map(|artist| {
                    let is_starred = artist.starred.is_some() || starred.contains(&artist.id);
                    crate::browse::rows::artist_json(
                        artist,
                        &covers,
                        None,
                        is_starred,
                        ratings.get(&artist.id).copied().unwrap_or(0),
                    )
                })
                .collect()
        }
        "songs" => {
            let songs = crate::library::songs_range(&dbs.library, start, count, order).await?;
            let covers =
                crate::covers::url_map_for(dbs, songs.iter().map(|song| &song.cover_art)).await?;
            songs
                .iter()
                .map(|song| {
                    crate::browse::rows::song_json(
                        &song.id,
                        &song.title,
                        song.artist.as_deref(),
                        song.album.as_deref(),
                        song.album_id.as_deref(),
                        song.track,
                        song.duration,
                        song.suffix.as_deref(),
                        crate::covers::url_for(song.cover_art.as_deref(), &covers),
                        song.starred.is_some(),
                    )
                })
                .collect()
        }
        other => bail!("unknown row surface: {other}"),
    };

    let _ = offline;
    Ok(serde_json::to_string(&json!({
        "start": start,
        "rows": rows,
    }))?)
}
