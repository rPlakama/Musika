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
        crate::model::Scope::Playlists => crate::library::PLAYLIST_ORDERS
            [order.min(crate::library::PLAYLIST_ORDERS.len() - 1)]
        .to_string(),
    }
}

pub(crate) async fn library_rows(
    scope: crate::model::Scope,
    order: usize,
    offline: bool,
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
            let artist_images = crate::covers::artist_image_urls(
                &dbs,
                &artists
                    .iter()
                    .map(|artist| {
                        (
                            artist.id.clone(),
                            artist.image_url.clone().unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .await?;
            Ok(artists
                .into_iter()
                .map(|artist| {
                    let local_image = artist_images.get(&artist.id).cloned();
                    Row {
                        item: crate::model::ItemMeta {
                            id: artist.id,
                            title: artist.name,
                            cover_url: local_image.unwrap_or_else(|| {
                                crate::covers::url_for(artist.cover_art.as_deref(), &covers)
                            }),
                            cover_remote: artist
                                .image_url
                                .clone()
                                .or_else(|| {
                                    artist
                                        .cover_art
                                        .as_deref()
                                        .and_then(|id| remotes.get(id))
                                        .cloned()
                                })
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
                    }
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
        Scope::Playlists => {
            let order = crate::library::PLAYLIST_ORDERS
                [order.min(crate::library::PLAYLIST_ORDERS.len() - 1)];
            let downloaded_only = offline || crate::library::is_downloaded_order(order);
            crate::playlists::list_filtered(&dbs, downloaded_only).await
        }
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
            let mut songs = crate::library::album_songs(&dbs.library, &descriptor.id).await?;
            crate::offline::retain_songs(&dbs, offline, &mut songs).await?;
            let shown_tracks = songs.len() as i64;
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
                    if offline {
                        shown_tracks
                    } else {
                        album.song_count
                    },
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
                // Hero surfaces paint the untouched original; the row covers
                // further down keep their thumbnail URLs.
                cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
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
            let mut albums =
                crate::library::artist_albums(&dbs.library, &descriptor.id, "Name").await?;
            if offline && let Some(filter) = crate::offline::load(&dbs, true).await? {
                albums.retain(|album| filter.contains_album(&album.id));
            }
            let shown_albums = albums.len() as i64;
            let covers = crate::covers::url_map_for(
                &dbs,
                albums
                    .iter()
                    .map(|album| &album.cover_art)
                    .chain(std::iter::once(&artist.cover_art)),
            )
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
            let artist_images = crate::covers::artist_image_urls(
                &dbs,
                &[(
                    artist.id.clone(),
                    artist.image_url.clone().unwrap_or_default(),
                )],
            )
            .await?;
            let local_artist_image = artist_images.get(&artist.id).cloned();
            let header = PageHeader {
                title: artist.name,
                subtitle: albums_label(if offline {
                    shown_albums
                } else {
                    artist.album_count
                }),
                meta: String::new(),
                // Hero surfaces paint the untouched original; the album rows
                // further down keep their thumbnail URLs.
                cover_url: local_artist_image.unwrap_or_else(|| {
                    crate::covers::url_for(artist.cover_art.as_deref(), &covers)
                }),
                cover_remote: artist.image_url.clone().unwrap_or(cover_remote),
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
        PageKind::Playlist => crate::playlists::page(&dbs, &descriptor.id, offline).await,
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
                cover_url: playlist.cover.clone(),
                cover_remote: playlist.cover_remote.clone(),
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
    offline: bool,
) -> anyhow::Result<(Vec<crate::player::Track>, usize)> {
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    crate::queue::library_queue(&dbs, song_id, order, offline).await
}

pub(crate) fn send_event(tx: &UnboundedSender<Event>, event: Event) {
    _ = tx.send(event);
}

/// Emits `DownloadSettled` after dropping the downloaded-catalog cache: the
/// ledger just changed, and a `Downloaded` / offline reload triggered by the
/// UI must see the new rows immediately rather than a stale 2 s snapshot.
pub(crate) fn send_download_settled(tx: &UnboundedSender<Event>) {
    crate::offline::clear_cache();
    send_event(tx, Event::DownloadSettled);
}

/// RAII guard that guarantees [`send_download_settled`] is emitted when a
/// download task exits on any branch (including early returns and cancellation).
pub(crate) struct DownloadSettledGuard {
    tx: UnboundedSender<Event>,
}

impl DownloadSettledGuard {
    pub(crate) fn new(tx: UnboundedSender<Event>) -> Self {
        Self { tx }
    }
}

impl Drop for DownloadSettledGuard {
    fn drop(&mut self) {
        send_download_settled(&self.tx);
    }
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

fn html_tag_len(after_lt: &str) -> Option<usize> {
    let mut chars = after_lt.chars();
    let first = chars.next()?;
    let valid_start = first.is_ascii_alphabetic()
        || first == '!'
        || (first == '/' && chars.next().is_some_and(|ch| ch.is_ascii_alphabetic()));
    if !valid_start {
        return None;
    }
    let close = after_lt.find('>')?;
    if after_lt[..close].contains('<') {
        return None;
    }
    Some(close + 1)
}

pub(crate) fn plain_notes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after_lt = &rest[lt + 1..];
        if let Some(tag_len) = html_tag_len(after_lt) {
            rest = &after_lt[tag_len..];
        } else {
            out.push('<');
            rest = after_lt;
        }
    }
    out.push_str(rest);
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_notes_preserves_literal_angle_brackets_and_strips_valid_tags() {
        assert_eq!(plain_notes("A <3 B and <b>bold</b>"), "A <3 B and bold");
        assert_eq!(
            plain_notes("Bio with trailing <unclosed tag"),
            "Bio with trailing <unclosed tag"
        );
        assert_eq!(
            plain_notes("<p>Hello <a href=\"https://example.test\">world</a> &amp; friends!</p>"),
            "Hello world & friends!"
        );
    }

    #[test]
    fn download_settled_guard_emits_on_early_drop() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        {
            let _guard = DownloadSettledGuard::new(tx);
        }
        assert!(matches!(rx.try_recv(), Ok(Event::DownloadSettled)));
    }
}
