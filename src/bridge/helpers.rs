//! Request plumbing shared by the bridge request families.

use super::*;

pub(crate) fn order_name(scope: crate::model::Scope, order: usize) -> String {
    match scope {
        crate::model::Scope::Albums | crate::model::Scope::Songs => {
            crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)].to_string()
        }
        crate::model::Scope::Artists => crate::library::ARTIST_ORDERS
            [order.min(crate::library::ARTIST_ORDERS.len() - 1)]
        .to_string(),
        crate::model::Scope::Playlists => "Name".to_string(),
    }
}

pub(crate) async fn library_rows(
    scope: crate::model::Scope,
    order: usize,
) -> anyhow::Result<Vec<crate::model::Row>> {
    use crate::model::{Row, RowKind, Scope};
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    match scope {
        Scope::Albums => {
            let order = crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)];
            let albums = crate::library::albums(&dbs.library, None, order).await?;
            let covers =
                crate::covers::url_map_for(&dbs, albums.iter().map(|album| &album.cover_art))
                    .await?;
            let remotes =
                crate::covers::remote_map_for(&dbs, albums.iter().map(|album| &album.cover_art))
                    .await?;
            Ok(albums
                .into_iter()
                .map(|album| Row {
                    item: crate::model::ItemMeta {
                        id: album.id.clone(),
                        title: album.title,
                        cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
                        cover_remote: album
                            .cover_art
                            .as_deref()
                            .and_then(|id| remotes.get(id))
                            .cloned()
                            .unwrap_or_default(),
                        album_id: album.id,
                    },
                    track: String::new(),
                    kind: RowKind::Album,
                    subtitle: format!(
                        "{}{}",
                        album.artist.unwrap_or_default(),
                        album
                            .year
                            .map(|year| format!(" · {year}"))
                            .unwrap_or_default()
                    ),
                    meta: if album.song_count == 1 {
                        "1 track".to_string()
                    } else {
                        format!("{} tracks", album.song_count)
                    },
                    song_id: String::new(),
                    starred: album.starred.is_some(),
                    rating: album.rating,
                })
                .collect())
        }
        Scope::Artists => {
            let order =
                crate::library::ARTIST_ORDERS[order.min(crate::library::ARTIST_ORDERS.len() - 1)];
            let artists = crate::library::artists(&dbs.library, order).await?;
            let covers =
                crate::covers::url_map_for(&dbs, artists.iter().map(|artist| &artist.cover_art))
                    .await?;
            let remotes =
                crate::covers::remote_map_for(&dbs, artists.iter().map(|artist| &artist.cover_art))
                    .await?;
            Ok(artists
                .into_iter()
                .map(|artist| Row {
                    item: crate::model::ItemMeta {
                        id: artist.id,
                        title: artist.name,
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
                    subtitle: albums_label(artist.album_count),
                    meta: String::new(),
                    song_id: String::new(),
                    starred: artist.starred.is_some(),
                    rating: 0,
                })
                .collect())
        }
        Scope::Songs => {
            let order = crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)];
            let songs = crate::library::all_songs(&dbs.library, order).await?;
            let covers =
                crate::covers::url_map_for(&dbs, songs.iter().map(|song| &song.cover_art)).await?;
            let remotes =
                crate::covers::remote_map_for(&dbs, songs.iter().map(|song| &song.cover_art))
                    .await?;
            Ok(songs
                .into_iter()
                .map(|song| Row {
                    item: crate::model::ItemMeta {
                        id: song.id.clone(),
                        title: song.title,
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
                    subtitle: song.artist.unwrap_or_default(),
                    meta: song.album.clone().unwrap_or_default(),
                    song_id: song.id,
                    starred: song.starred.is_some(),
                    rating: 0,
                })
                .collect())
        }
        Scope::Playlists => crate::playlists::list(&dbs).await,
    }
}

pub(crate) fn songs_label(count: i64) -> String {
    if count == 1 {
        "1 song".to_string()
    } else {
        format!("{count} songs")
    }
}

pub(crate) async fn page_data(
    descriptor: &crate::model::PageDescriptor,
    offline: bool,
) -> anyhow::Result<(crate::model::PageHeader, Vec<crate::model::Row>)> {
    use crate::model::{PageHeader, PageKind, Row, RowKind};
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    match descriptor.kind {
        PageKind::Album => {
            let album = crate::library::album(&dbs.library, &descriptor.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("album not found"))?;
            let songs = crate::library::album_songs(&dbs.library, &descriptor.id).await?;
            let mut ids: Vec<&Option<String>> = songs.iter().map(|song| &song.cover_art).collect();
            ids.push(&album.cover_art);
            let covers = crate::covers::url_map_for(&dbs, ids.iter().copied()).await?;
            let remotes = crate::covers::remote_map_for(&dbs, ids.iter().copied()).await?;
            let cover_remote = album
                .cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default();
            let header = PageHeader {
                title: album.title,
                subtitle: album.artist.unwrap_or_default(),
                meta: format!(
                    "{} Tracks{}{}",
                    album.song_count,
                    album
                        .year
                        .map(|year| format!(" · {year}"))
                        .unwrap_or_default(),
                    if album.duration > 0 {
                        format!(" · {} Minutes", (album.duration + 59) / 60)
                    } else {
                        String::new()
                    }
                ),
                cover_url: thumbify(crate::covers::url_for(album.cover_art.as_deref(), &covers)),
                cover_remote,
                collage: Vec::new(),
                collage_remote: Vec::new(),
                collage_art: Vec::new(),
                cover_art: album.cover_art.unwrap_or_default(),
                rating: album.rating,
                starred: album.starred.is_some(),
            };
            let rows = songs
                .into_iter()
                .map(|song| Row {
                    item: crate::model::ItemMeta {
                        id: song.id.clone(),
                        title: song.title,
                        cover_url: crate::covers::url_for(song.cover_art.as_deref(), &covers),
                        cover_remote: song
                            .cover_art
                            .as_deref()
                            .and_then(|id| remotes.get(id))
                            .cloned()
                            .unwrap_or_default(),
                        album_id: descriptor.id.clone(),
                    },
                    track: song
                        .track
                        .map(|track| track.to_string())
                        .unwrap_or_default(),
                    kind: RowKind::Song,
                    subtitle: song.artist.unwrap_or_default(),
                    meta: song
                        .duration
                        .map(|seconds| crate::browse::rows::format_duration(Some(seconds)))
                        .unwrap_or_default(),
                    song_id: song.id,
                    starred: song.starred.is_some(),
                    rating: 0,
                })
                .collect();
            Ok((header, rows))
        }
        PageKind::Artist => {
            let artist = crate::library::artist(&dbs.library, &descriptor.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("artist not found"))?;
            let albums =
                crate::library::artist_albums(&dbs.library, &descriptor.id, "Name").await?;
            let covers =
                crate::covers::url_map_for(&dbs, albums.iter().map(|album| &album.cover_art))
                    .await?;
            let remotes = crate::covers::remote_map_for(
                &dbs,
                albums
                    .iter()
                    .map(|album| &album.cover_art)
                    .chain(std::iter::once(&artist.cover_art)),
            )
            .await?;
            let cover_remote = artist
                .cover_art
                .as_deref()
                .and_then(|id| remotes.get(id))
                .cloned()
                .unwrap_or_default();
            let header = PageHeader {
                title: artist.name,
                subtitle: albums_label(artist.album_count),
                meta: String::new(),
                cover_url: thumbify(crate::covers::url_for(artist.cover_art.as_deref(), &covers)),
                cover_remote,
                collage: Vec::new(),
                collage_remote: Vec::new(),
                collage_art: Vec::new(),
                cover_art: artist.cover_art.unwrap_or_default(),
                rating: 0,
                starred: artist.starred.is_some(),
            };
            let rows = albums
                .into_iter()
                .map(|album| Row {
                    item: crate::model::ItemMeta {
                        id: album.id.clone(),
                        title: album.title,
                        cover_url: thumbify(crate::covers::url_for(
                            album.cover_art.as_deref(),
                            &covers,
                        )),
                        cover_remote: album
                            .cover_art
                            .as_deref()
                            .and_then(|id| remotes.get(id))
                            .cloned()
                            .unwrap_or_default(),
                        album_id: album.id,
                    },
                    track: String::new(),
                    kind: RowKind::Album,
                    subtitle: album.artist.unwrap_or_default(),
                    meta: format!(
                        "{} · {}",
                        songs_label(album.song_count),
                        crate::browse::rows::format_duration(Some(album.duration as i32))
                    ),
                    song_id: String::new(),
                    starred: album.starred.is_some(),
                    rating: album.rating,
                })
                .collect();
            Ok((header, rows))
        }
        PageKind::Radio => {
            let seed_kind = descriptor.radio_seed_kind.as_deref().unwrap_or("album");
            crate::radio::page(&dbs, seed_kind, &descriptor.id, offline).await
        }
        PageKind::Playlist => crate::playlists::page(&dbs, &descriptor.id).await,
    }
}

pub(crate) async fn search_rows(
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<(Vec<crate::model::Row>, bool)> {
    use crate::model::{Row, RowKind};

    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    let results = crate::browse::search(&dbs, query, offline, limit.max(1)).await?;
    let most_played = results.most_played;
    let mut rows = Vec::new();

    for artist in results.artists.iter().take(limit) {
        rows.push(Row {
            item: crate::model::ItemMeta {
                id: artist.id.clone(),
                title: artist.name.clone(),
                cover_url: artist.cover.clone(),
                cover_remote: artist.cover_remote.clone(),
                album_id: String::new(),
            },
            track: String::new(),
            kind: RowKind::Artist,
            subtitle: artist.meta(),
            meta: String::new(),
            song_id: String::new(),
            starred: artist.starred,
            rating: artist.rating,
        });
    }
    for album in results.albums.iter().take(limit) {
        let meta = if album.year > 0 {
            format!("{} · {}", album.year, album.meta())
        } else {
            album.meta()
        };
        rows.push(Row {
            item: crate::model::ItemMeta {
                id: album.id.clone(),
                title: album.title.clone(),
                cover_url: album.cover.clone(),
                cover_remote: album.cover_remote.clone(),
                album_id: album.id.clone(),
            },
            track: String::new(),
            kind: RowKind::Album,
            subtitle: album.artist.clone(),
            meta,
            song_id: String::new(),
            starred: album.starred,
            rating: album.rating,
        });
    }
    for song in results.songs.iter().take(limit) {
        rows.push(Row {
            item: crate::model::ItemMeta {
                id: song.id.clone(),
                title: song.title.clone(),
                cover_url: song.cover.clone(),
                cover_remote: song.cover_remote.clone(),
                album_id: song.album_id.clone().unwrap_or_default(),
            },
            track: song
                .track
                .map(|track| track.to_string())
                .unwrap_or_default(),
            kind: RowKind::Song,
            subtitle: song.subtitle(),
            meta: if most_played {
                if song.play_count == 1 {
                    "1 play".to_string()
                } else {
                    format!("{} plays", song.play_count)
                }
            } else {
                song.meta()
            },
            song_id: song.id.clone(),
            starred: song.starred,
            rating: 0,
        });
    }
    for playlist in results.playlists.iter().take(limit) {
        rows.push(Row {
            item: crate::model::ItemMeta {
                id: playlist.id.clone(),
                title: playlist.name.clone(),
                cover_url: String::new(),
                cover_remote: String::new(),
                album_id: String::new(),
            },
            track: String::new(),
            kind: RowKind::Playlist,
            subtitle: playlist.subtitle.clone(),
            meta: playlist.meta.clone(),
            song_id: String::new(),
            starred: false,
            rating: 0,
        });
    }
    Ok((rows, most_played))
}

pub(crate) async fn queue_for_song(
    song_id: &str,
    order: &str,
) -> anyhow::Result<(Vec<crate::player::Track>, usize)> {
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    crate::queue::library_queue(&dbs, song_id, order, false).await
}

pub(crate) fn unix_ms_for_cover() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

pub(crate) fn send_event(tx: &UnboundedSender<Event>, event: Event) {
    let _ = tx.send(event);
}

pub(crate) fn emit_queue(
    tx: &UnboundedSender<Event>,
    result: Result<Vec<crate::player::Track>, anyhow::Error>,
    start: usize,
    context: &str,
) {
    match result {
        Ok(tracks) => send_event(tx, Event::Queue { tracks, start }),
        Err(err) => send_event(tx, Event::Error(format!("{context}: {err:#}"))),
    }
}

pub(crate) fn emit_queue_pair(
    tx: &UnboundedSender<Event>,
    result: Result<(Vec<crate::player::Track>, usize), anyhow::Error>,
    context: &str,
) {
    match result {
        Ok((tracks, start)) => emit_queue(tx, Ok(tracks), start, context),
        Err(err) => emit_queue(tx, Err(err), 0, context),
    }
}

pub(crate) fn emit_append(
    tx: &UnboundedSender<Event>,
    result: Result<Vec<crate::player::Track>, anyhow::Error>,
    context: &str,
) {
    match result {
        Ok(tracks) => send_event(tx, Event::QueueAppend { tracks }),
        Err(err) => send_event(tx, Event::Error(format!("{context}: {err:#}"))),
    }
}

pub(crate) fn collage_urls(value: &serde_json::Value) -> Vec<String> {
    value["collage"]
        .as_array()
        .map(|collage| {
            collage
                .iter()
                .filter_map(|cover| cover.as_str())
                .map(str::trim)
                .filter(|cover| !cover.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn collage_header(value: &serde_json::Value, title_key: &str) -> PageHeader {
    let collage = collage_urls(value);
    PageHeader {
        title: value[title_key].as_str().unwrap_or_default().to_string(),
        subtitle: value["subtitle"].as_str().unwrap_or_default().to_string(),
        meta: value["meta"].as_str().unwrap_or_default().to_string(),
        cover_url: collage.first().cloned().unwrap_or_default(),
        cover_remote: value["collage_remote"]
            .as_array()
            .and_then(|remotes| remotes.first())
            .and_then(|remote| remote.as_str())
            .unwrap_or_default()
            .to_string(),
        collage_art: value["collage_art"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .map(|id| id.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        collage,
        collage_remote: value["collage_remote"]
            .as_array()
            .map(|remotes| {
                remotes
                    .iter()
                    .map(|remote| remote.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        cover_art: String::new(),
        rating: 0,
        starred: false,
    }
}

pub(crate) fn thumbify(url: String) -> String {
    if url.is_empty()
        || url.contains(".thumb.")
        || url.contains("#no-thumb")
        || !url.contains(".jpg")
    {
        return url;
    }
    url.replacen(".jpg", ".thumb.jpg", 1)
}

pub(crate) fn plain_notes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}
