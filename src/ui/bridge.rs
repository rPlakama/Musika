// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc::UnboundedSender;

pub enum Event {
    LoginRequired {
        offline_available: bool,
    },
    LoggedIn {
        server_url: String,
        username: String,
        offline: bool,
    },
    AuthError(String),
    Library {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        rows: Vec<super::state::Row>,
    },
    Page {
        generation: u64,
        descriptor: super::state::PageDescriptor,
        header: super::state::PageHeader,
        rows: Vec<super::state::Row>,
    },
    Queue {
        tracks: Vec<crate::player::Track>,
        start: usize,
    },
    Search {
        generation: u64,
        query: String,
        rows: Vec<super::state::Row>,
    },
    Header {
        server_url: String,
        username: String,
        last_sync: Option<String>,
        counts: (i64, i64, i64),
    },
    Error(String),
}

pub struct Bridge {
    tx: UnboundedSender<Event>,
    generation: AtomicU64,
    page_generation: AtomicU64,
    search_generation: AtomicU64,
}

impl Bridge {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        let bridge = Self {
            tx,
            generation: AtomicU64::new(0),
            page_generation: AtomicU64::new(0),
            search_generation: AtomicU64::new(0),
        };
        bridge.refresh_header();
        bridge
    }

    pub fn refresh_header(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match crate::app::dbs() {
                Some(dbs) => match crate::sqlite::load_credentials(&dbs.credentials).await {
                    Ok(Some(credentials)) => {
                        let status = crate::library::status(&dbs.library).await.ok();
                        Event::Header {
                            server_url: credentials.server_url,
                            username: credentials.username,
                            last_sync: status.as_ref().and_then(|status| status.last_sync.clone()),
                            counts: status
                                .map(|status| (status.artists, status.albums, status.songs))
                                .unwrap_or((0, 0, 0)),
                        }
                    }
                    Ok(None) => {
                        let offline_available = crate::auth::downloaded_available().await;
                        return send_event(&tx, Event::LoginRequired { offline_available });
                    }
                    Err(err) => Event::Error(format!("Could not read credentials: {err:#}")),
                },
                None => Event::Error("The library is not initialised".to_string()),
            };
            let _ = tx.send(event);
        });
    }

    pub fn load_page(&self, descriptor: super::state::PageDescriptor) {
        let generation = self.page_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match page_data(&descriptor).await {
                Ok((header, mut rows)) => {
                    mark_downloaded(&mut rows).await;
                    crate::debug_log!(
                        "GPUI: {} page {} rows",
                        descriptor.kind.label(),
                        rows.len()
                    );
                    Event::Page {
                        generation,
                        descriptor,
                        header,
                        rows,
                    }
                }
                Err(err) => Event::Error(format!("Page load failed: {err:#}")),
            };
            send_event(&tx, event);
        });
    }

    pub fn play_song(&self, song_id: String, order: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match queue_for_song(&song_id, &order).await {
                Ok((tracks, start)) => {
                    crate::debug_log!("GPUI: queue {} tracks start {}", tracks.len(), start);
                    Event::Queue { tracks, start }
                }
                Err(err) => Event::Error(format!("Playback failed: {err:#}")),
            };
            let _ = tx.send(event);
        });
    }

    pub fn login(&self, server_url: String, username: String, password: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match crate::auth::login(&server_url, &username, &password).await {
                Ok(account) => {
                    crate::debug_log!(
                        "GPUI: logged in as {} @ {} ({})",
                        account.username,
                        account.server_url,
                        account.server_type
                    );
                    Event::LoggedIn {
                        server_url: account.server_url,
                        username: account.username,
                        offline: false,
                    }
                }
                Err(err) => Event::AuthError(format!("{err:#}")),
            };
            send_event(&tx, event);
        });
    }

    pub fn offline_login(&self, server_url: String, username: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let account = crate::auth::offline(&server_url, &username).await;
            crate::debug_log!("GPUI: offline login as {} @ {}", account.username, account.server_url);
            send_event(
                &tx,
                Event::LoggedIn {
                    server_url: account.server_url,
                    username: account.username,
                    offline: true,
                },
            );
        });
    }

    pub fn search(&self, query: String) {
        let generation = self.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match search_rows(&query).await {
                Ok(rows) => {
                    crate::debug_log!("GPUI: search {:?} {} rows", query, rows.len());
                    Event::Search {
                        generation,
                        query,
                        rows,
                    }
                }
                Err(err) => Event::Error(format!("Search failed: {err:#}")),
            };
            let _ = tx.send(event);
        });
    }

    pub fn load_library(&self, scope: super::state::Scope, order: usize) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match library_rows(scope, order).await {
                Ok(mut rows) => {
                    mark_downloaded(&mut rows).await;
                    crate::debug_log!("GPUI: {} {} rows", scope.label(), rows.len());
                    Event::Library {
                        generation,
                        scope,
                        order,
                        rows,
                    }
                }
                Err(err) => Event::Error(format!("Library load failed: {err:#}")),
            };
            let _ = tx.send(event);
        });
    }
}

async fn library_rows(
    scope: super::state::Scope,
    order: usize,
) -> anyhow::Result<Vec<super::state::Row>> {
    use super::state::{Row, RowKind, Scope};
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    match scope {
        Scope::Albums => {
            let order = crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)];
            let albums = crate::library::albums(&dbs.library, None, order).await?;
            let covers = crate::covers::url_map_for(
                &dbs,
                albums.iter().map(|album| &album.cover_art),
            )
            .await?;
            Ok(albums
                .into_iter()
                .map(|album| Row {
                    id: album.id.clone(),
                    kind: RowKind::Album,
                    title: album.title,
                    subtitle: album.artist.unwrap_or_default(),
                    meta: format!(
                        "{} · {}",
                        songs_label(album.song_count),
                        crate::browse::rows::format_duration(Some(album.duration as i32))
                    ),
                    cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
                    song_id: String::new(),
                    album_id: album.id,
                    starred: album.starred.is_some(),
                    rating: album.rating,
                    downloaded: false,
                })
                .collect())
        }
        Scope::Artists => {
            let order = crate::library::ARTIST_ORDERS[order.min(crate::library::ARTIST_ORDERS.len() - 1)];
            let artists = crate::library::artists(&dbs.library, order).await?;
            let covers = crate::covers::url_map_for(
                &dbs,
                artists.iter().map(|artist| &artist.cover_art),
            )
            .await?;
            Ok(artists
                .into_iter()
                .map(|artist| Row {
                    id: artist.id,
                    kind: RowKind::Artist,
                    title: artist.name,
                    subtitle: format!("{} albums", artist.album_count),
                    meta: String::new(),
                    cover_url: crate::covers::url_for(artist.cover_art.as_deref(), &covers),
                    song_id: String::new(),
                    album_id: String::new(),
                    starred: artist.starred.is_some(),
                    rating: 0,
                    downloaded: false,
                })
                .collect())
        }
        Scope::Songs => {
            let order = crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)];
            let songs = crate::library::all_songs(&dbs.library, order).await?;
            let covers = crate::covers::url_map_for(
                &dbs,
                songs.iter().map(|song| &song.cover_art),
            )
            .await?;
            Ok(songs
                .into_iter()
                .map(|song| Row {
                    id: song.id.clone(),
                    kind: RowKind::Song,
                    title: song.title,
                    subtitle: song.artist.unwrap_or_default(),
                    meta: song.album.clone().unwrap_or_default(),
                    cover_url: crate::covers::url_for(song.cover_art.as_deref(), &covers),
                    song_id: song.id,
                    album_id: song.album_id.unwrap_or_default(),
                    starred: song.starred.is_some(),
                    rating: 0,
                    downloaded: false,
                })
                .collect())
        }
        Scope::Playlists => {
            let raw = crate::playlists::list_json(&dbs).await?;
            let values: Vec<serde_json::Value> = serde_json::from_str(&raw)?;
            Ok(values
                .into_iter()
                .map(|value| Row {
                    id: value["id"].as_str().unwrap_or_default().to_string(),
                    kind: RowKind::Playlist,
                    title: value["name"].as_str().unwrap_or_default().to_string(),
                    subtitle: value["subtitle"].as_str().unwrap_or_default().to_string(),
                    meta: value["meta"].as_str().unwrap_or_default().to_string(),
                    cover_url: String::new(),
                    song_id: String::new(),
                    album_id: String::new(),
                    starred: false,
                    rating: 0,
                    downloaded: false,
                })
                .collect())
        }
    }
}

async fn mark_downloaded(rows: &mut [super::state::Row]) {
    let Some(dbs) = crate::app::dbs() else { return };
    let ids: Vec<String> = rows
        .iter()
        .filter(|row| !row.song_id.is_empty())
        .map(|row| row.song_id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let Ok(done) = crate::download::done_ids_of(&dbs.download, &ids).await else {
        return;
    };
    let done: std::collections::HashSet<String> = done.into_iter().collect();
    for row in rows {
        row.downloaded = done.contains(&row.song_id);
    }
}

fn songs_label(count: i64) -> String {
    if count == 1 { "1 song".to_string() } else { format!("{count} songs") }
}

async fn page_data(
    descriptor: &super::state::PageDescriptor,
) -> anyhow::Result<(super::state::PageHeader, Vec<super::state::Row>)> {
    use super::state::{PageHeader, PageKind, Row, RowKind};
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    match descriptor.kind {
        PageKind::Album => {
            let album = crate::library::album(&dbs.library, &descriptor.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("album not found"))?;
            let songs = crate::library::album_songs(&dbs.library, &descriptor.id).await?;
            let mut ids: Vec<&Option<String>> = songs.iter().map(|song| &song.cover_art).collect();
            ids.push(&album.cover_art);
            let covers = crate::covers::url_map_for(&dbs, ids).await?;
            let header = PageHeader {
                title: album.title,
                subtitle: album.artist.unwrap_or_default(),
                meta: format!(
                    "{} · {}",
                    songs_label(album.song_count),
                    crate::browse::rows::format_duration(Some(album.duration as i32))
                ),
                cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
            };
            let rows = songs
                .into_iter()
                .map(|song| Row {
                    id: song.id.clone(),
                    kind: RowKind::Song,
                    title: song.title,
                    subtitle: song.artist.unwrap_or_default(),
                    meta: song
                        .duration
                        .map(|seconds| crate::browse::rows::format_duration(Some(seconds)))
                        .unwrap_or_default(),
                    cover_url: crate::covers::url_for(song.cover_art.as_deref(), &covers),
                    song_id: song.id,
                    album_id: descriptor.id.clone(),
                    starred: song.starred.is_some(),
                    rating: 0,
                    downloaded: false,
                })
                .collect();
            Ok((header, rows))
        }
        PageKind::Artist => {
            let artist = crate::library::artist(&dbs.library, &descriptor.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("artist not found"))?;
            let albums = crate::library::albums(&dbs.library, Some(&descriptor.id), "Name").await?;
            let covers = crate::covers::url_map_for(
                &dbs,
                albums.iter().map(|album| &album.cover_art),
            )
            .await?;
            let header = PageHeader {
                title: artist.name,
                subtitle: format!("{} albums", artist.album_count),
                meta: String::new(),
                cover_url: crate::covers::url_for(artist.cover_art.as_deref(), &covers),
            };
            let rows = albums
                .into_iter()
                .map(|album| Row {
                    id: album.id.clone(),
                    kind: RowKind::Album,
                    title: album.title,
                    subtitle: album.artist.unwrap_or_default(),
                    meta: format!(
                        "{} · {}",
                        songs_label(album.song_count),
                        crate::browse::rows::format_duration(Some(album.duration as i32))
                    ),
                    cover_url: crate::covers::url_for(album.cover_art.as_deref(), &covers),
                    song_id: String::new(),
                    album_id: album.id,
                    starred: album.starred.is_some(),
                    rating: album.rating,
                    downloaded: false,
                })
                .collect();
            Ok((header, rows))
        }
        PageKind::Playlist => {
            let raw = crate::playlists::page_json(&dbs, &descriptor.id, false).await?;
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            let header = PageHeader {
                title: value["name"].as_str().unwrap_or_default().to_string(),
                subtitle: value["subtitle"].as_str().unwrap_or_default().to_string(),
                meta: value["meta"].as_str().unwrap_or_default().to_string(),
                cover_url: value["collage"]
                    .as_array()
                    .and_then(|collage| collage.first())
                    .and_then(|cover| cover.as_str())
                    .unwrap_or_default()
                    .to_string(),
            };
            let rows = value["songs"]
                .as_array()
                .map(|songs| {
                    songs
                        .iter()
                        .map(|song| Row {
                            id: song["songId"].as_str().unwrap_or_default().to_string(),
                            kind: RowKind::Song,
                            title: song["title"].as_str().unwrap_or_default().to_string(),
                            subtitle: song["subtitle"].as_str().unwrap_or_default().to_string(),
                            meta: song["duration"].as_str().unwrap_or_default().to_string(),
                            cover_url: song["cover"].as_str().unwrap_or_default().to_string(),
                            song_id: song["songId"].as_str().unwrap_or_default().to_string(),
                            album_id: song["albumId"].as_str().unwrap_or_default().to_string(),
                            starred: song["starred"].as_bool().unwrap_or(false),
                            rating: 0,
                    downloaded: false,
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok((header, rows))
        }
    }
}

async fn search_rows(query: &str) -> anyhow::Result<Vec<super::state::Row>> {
    use super::state::{Row, RowKind};
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    let index = crate::search::index::ensure(&dbs.library).await?;
    let Some(index) = index else {
        return Ok(Vec::new());
    };
    let results = index.search(query, 40);
    let mut rows = Vec::new();
    for album in results.albums {
        rows.push(Row {
            id: album.id.clone(),
            kind: RowKind::Album,
            title: album.title.clone(),
            subtitle: album.artist.clone().unwrap_or_default(),
            meta: format!("{} · {}", album.year.unwrap_or_default(), songs_label(album.song_count)),
            cover_url: crate::covers::url_for(album.cover_art.as_deref(), &Default::default()),
            song_id: String::new(),
            album_id: album.id.clone(),
            starred: album.starred.is_some(),
            rating: album.rating,
            downloaded: false,
        });
    }
    for artist in results.artists {
        rows.push(Row {
            id: artist.id.clone(),
            kind: RowKind::Artist,
            title: artist.name.clone(),
            subtitle: format!("{} albums", artist.album_count),
            meta: String::new(),
            cover_url: String::new(),
            song_id: String::new(),
            album_id: String::new(),
            starred: artist.starred.is_some(),
            rating: 0,
            downloaded: false,
        });
    }
    for song in results.songs {
        rows.push(Row {
            id: song.id.clone(),
            kind: RowKind::Song,
            title: song.title.clone(),
            subtitle: song.artist.clone().unwrap_or_default(),
            meta: song.album.clone().unwrap_or_default(),
            cover_url: String::new(),
            song_id: song.id.clone(),
            album_id: song.album_id.clone().unwrap_or_default(),
            starred: false,
            rating: 0,
            downloaded: false,
        });
    }
    Ok(rows)
}

async fn queue_for_song(
    song_id: &str,
    order: &str,
) -> anyhow::Result<(Vec<crate::player::Track>, usize)> {
    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    let (json, start) = crate::queue::library_queue_payload(&dbs, song_id, order, false).await?;
    let tracks = serde_json::from_str::<Vec<crate::player::Track>>(&json)?;
    Ok((tracks, start))
}

fn send_event(tx: &UnboundedSender<Event>, event: Event) {
    let _ = tx.send(event);
}
