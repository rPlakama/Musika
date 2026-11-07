// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::mpsc::UnboundedSender;

mod page;
mod library;
mod playback;
mod radio;
mod actions;
mod covers;
mod maintenance;
mod auth;
mod marks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    Incremental,
    All,
    Repair,
}

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
    LibraryCount {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        count: usize,
    },
    LibraryWindow {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        start: usize,
        rows: Vec<super::state::Row>,
    },
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
    Notes {
        album_id: String,
        notes: String,
    },
    Marks {
        generation: u64,
        result: crate::download::MarkWindowResult,
    },
    Preload {
        scope: super::state::Scope,
        order: usize,
        count: usize,
        rows: Vec<super::state::Row>,
    },
    DisplayCover {
        key: String,
        cover_art: String,
        target: u32,
        path: Option<std::path::PathBuf>,
    },
    RowsPatched {
        kind: super::state::RowKind,
        id: String,
        starred: Option<bool>,
        rating: Option<u8>,
    },
    AlbumState {
        id: String,
        starred: bool,
        rating: u8,
    },
    LibrarySynced,
    RadioSaved {
        name: String,
        remote_error: Option<String>,
    },
    SyncProgress {
        message: String,
        done: usize,
        total: usize,
    },
    PageContainerState {
        kind: super::state::PageKind,
        id: String,
        cached: bool,
        starred: bool,
        rating: u8,
    },
    Wiped {
        scope: String,
    },
    QueueAppend {
        tracks: Vec<crate::player::Track>,
    },
    DownloadSettled,
    Palette {
        cover_art: String,
        roles: BTreeMap<String, String>,
    },
    PaletteReset {
        cover_art: String,
    },
    OpenPage {
        descriptor: super::state::PageDescriptor,
    },
    OpenPageReveal {
        descriptor: super::state::PageDescriptor,
        row_kind: super::state::RowKind,
        row_id: String,
    },
    Command(String),
    Restore {
        volume: f64,
        muted: bool,
        tracks: Vec<crate::player::Track>,
        start: usize,
        position: f64,
        page_kind: String,
        page_id: String,
    },
    Lyrics {
        song_id: String,
        document: crate::lyrics::Document,
    },
    Queue {
        tracks: Vec<crate::player::Track>,
        start: usize,
    },
    Search {
        generation: u64,
        query: String,
        rows: Vec<super::state::Row>,
        most_played: bool,
    },
    Header {
        server_url: String,
        username: String,
        server_type: String,
        last_sync: Option<String>,
        counts: (i64, i64, i64),
    },
    Error(String),
}

pub struct Bridge {
    tx: UnboundedSender<Event>,
    generation: AtomicU64,
    page_generation: AtomicU64,
    search_generation: Arc<AtomicU64>,
    marks_generation: AtomicU64,
    offline: AtomicBool,
    palette_inflight: Arc<Mutex<Option<String>>>,
    display_inflight: Arc<Mutex<HashSet<String>>>,
}

impl Bridge {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        let bridge = Self {
            tx,
            generation: AtomicU64::new(0),
            page_generation: AtomicU64::new(0),
            search_generation: Arc::new(AtomicU64::new(0)),
            marks_generation: AtomicU64::new(0),
            offline: AtomicBool::new(false),
            palette_inflight: Arc::new(Mutex::new(None)),
            display_inflight: Arc::new(Mutex::new(HashSet::new())),
        };
        bridge.refresh_header();
        bridge
    }






















    /// Fullscreen cover focus: fetch the playing album's favourite/rating.
    pub fn request_album_state(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let state = crate::library::album_states(&dbs.library, std::slice::from_ref(&album_id))
                .await
                .ok()
                .and_then(|states| states.get(&album_id).copied());
            if let Some((starred, rating)) = state {
                let _ = tx.send(Event::AlbumState {
                    id: album_id,
                    starred,
                    rating: rating.clamp(0, 5) as u8,
                });
            }
        });
    }


















}

fn order_name(scope: super::state::Scope, order: usize) -> String {
    match scope {
        super::state::Scope::Albums | super::state::Scope::Songs => {
            crate::library::ORDERS[order.min(crate::library::ORDERS.len() - 1)].to_string()
        }
        super::state::Scope::Artists => {
            crate::library::ARTIST_ORDERS[order.min(crate::library::ARTIST_ORDERS.len() - 1)]
                .to_string()
        }
        super::state::Scope::Playlists => "Name".to_string(),
    }
}

fn row_from_json(scope: super::state::Scope, value: &serde_json::Value) -> super::state::Row {
    use super::state::{Row, RowKind, Scope};
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    let kind = match scope {
        Scope::Albums => RowKind::Album,
        Scope::Artists => RowKind::Artist,
        Scope::Songs => RowKind::Song,
        Scope::Playlists => RowKind::Playlist,
    };
    Row {
        track: String::new(),
        id: text("id"),
        kind,
        title: text("title"),
        subtitle: text("subtitle"),
        meta: text("meta"),
        cover_url: text("cover"),
        song_id: text("songId"),
        album_id: text("albumId"),
        starred: value["starred"].as_bool().unwrap_or(false),
        rating: value["rating"].as_i64().unwrap_or(0),
        downloaded: false,
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
                    track: String::new(),
                    id: album.id.clone(),
                    kind: RowKind::Album,
                    title: album.title,
                    subtitle: format!(
                        "{}{}",
                        album.artist.unwrap_or_default(),
                        album.year.map(|year| format!(" · {year}")).unwrap_or_default()
                    ),
                    meta: if album.song_count == 1 { "1 track".to_string() } else { format!("{} tracks", album.song_count) },
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
                    track: String::new(),
                    id: artist.id,
                    kind: RowKind::Artist,
                    title: artist.name,
                    subtitle: albums_label(artist.album_count),
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
                    track: String::new(),
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
                    track: String::new(),
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

fn songs_label(count: i64) -> String {
    if count == 1 { "1 song".to_string() } else { format!("{count} songs") }
}

pub(crate) fn albums_label(count: i64) -> String {
    if count == 1 { "1 album".to_string() } else { format!("{count} albums") }
}

async fn page_data(
    descriptor: &super::state::PageDescriptor,
    offline: bool,
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
                    "{} Tracks{}{}",
                    album.song_count,
                    album.year.map(|year| format!(" · {year}")).unwrap_or_default(),
                    if album.duration > 0 { format!(" · {} Minutes", (album.duration + 59) / 60) } else { String::new() }
                ),
                cover_url: thumbify(crate::covers::url_for(album.cover_art.as_deref(), &covers)),
                collage: Vec::new(),
                cover_art: album.cover_art.unwrap_or_default(),
                rating: album.rating,
                starred: album.starred.is_some(),
            };
            let rows = songs
                .into_iter()
                .map(|song| Row {
                    track: song.track.map(|track| track.to_string()).unwrap_or_default(),
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
            let albums = crate::library::artist_albums(&dbs.library, &descriptor.id, "Name").await?;
            let covers = crate::covers::url_map_for(
                &dbs,
                albums.iter().map(|album| &album.cover_art),
            )
            .await?;
            let header = PageHeader {
                title: artist.name,
                subtitle: albums_label(artist.album_count),
                meta: String::new(),
                cover_url: thumbify(crate::covers::url_for(artist.cover_art.as_deref(), &covers)),
                collage: Vec::new(),
                cover_art: artist.cover_art.unwrap_or_default(),
                rating: 0,
                starred: artist.starred.is_some(),
            };
            let rows = albums
                .into_iter()
                .map(|album| Row {
                    track: String::new(),
                    id: album.id.clone(),
                    kind: RowKind::Album,
                    title: album.title,
                    subtitle: album.artist.unwrap_or_default(),
                    meta: format!(
                        "{} · {}",
                        songs_label(album.song_count),
                        crate::browse::rows::format_duration(Some(album.duration as i32))
                    ),
                    cover_url: thumbify(crate::covers::url_for(album.cover_art.as_deref(), &covers)),
                    song_id: String::new(),
                    album_id: album.id,
                    starred: album.starred.is_some(),
                    rating: album.rating,
                    downloaded: false,
                })
                .collect();
            Ok((header, rows))
        }
        PageKind::Radio => {
            let seed_kind = descriptor.radio_seed_kind.as_deref().unwrap_or("album");
            let raw = crate::radio::page_json(&dbs, seed_kind, &descriptor.id, offline).await?;
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            let header = PageHeader {
                title: value["title"].as_str().unwrap_or_default().to_string(),
                subtitle: value["subtitle"].as_str().unwrap_or_default().to_string(),
                meta: value["meta"].as_str().unwrap_or_default().to_string(),
                cover_url: value["collage"]
                    .as_array()
                    .and_then(|collage| collage.first())
                    .and_then(|cover| cover.as_str())
                    .unwrap_or_default()
                    .to_string(),
                collage: value["collage"]
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
                    .unwrap_or_default(),
                cover_art: String::new(),
                rating: 0,
                starred: false,
            };
            let rows = value["songs"]
                .as_array()
                .map(|songs| {
                    songs
                        .iter()
                        .map(|song| Row {
                            track: String::new(),
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
                collage: value["collage"]
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
                    .unwrap_or_default(),
                cover_art: String::new(),
                rating: 0,
                starred: false,
            };
            let rows = value["songs"]
                .as_array()
                .map(|songs| {
                    songs
                        .iter()
                        .map(|song| Row {
                            track: String::new(),
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

async fn search_rows(
    query: &str,
    offline: bool,
    limit: usize,
) -> anyhow::Result<(Vec<super::state::Row>, bool)> {
    use super::state::{Row, RowKind};

    let dbs = crate::app::dbs().ok_or_else(|| anyhow::anyhow!("databases not ready"))?;
    let json = crate::browse::search_json(&dbs, query, offline, limit.max(1)).await?;
    let value: serde_json::Value = serde_json::from_str(&json)?;
    let most_played = value["most_played"].as_bool().unwrap_or(false);
    let text = |row: &serde_json::Value, key: &str| {
        row[key].as_str().unwrap_or_default().to_string()
    };
    let mut rows = Vec::new();

    if let Some(artists) = value["artists"].as_array() {
        for artist in artists.iter().take(limit) {
            rows.push(Row {
                track: String::new(),
                id: text(artist, "id"),
                kind: RowKind::Artist,
                title: text(artist, "name"),
                subtitle: text(artist, "meta"),
                meta: String::new(),
                cover_url: text(artist, "cover"),
                song_id: String::new(),
                album_id: String::new(),
                starred: artist["starred"].as_bool().unwrap_or(false),
                rating: artist["rating"].as_i64().unwrap_or(0),
                downloaded: false,
            });
        }
    }
    if let Some(albums) = value["albums"].as_array() {
        for album in albums.iter().take(limit) {
            let year = album["year"].as_i64().unwrap_or(0);
            let meta = text(album, "meta");
            let meta = if year > 0 {
                format!("{year} · {meta}")
            } else {
                meta
            };
            rows.push(Row {
                track: String::new(),
                id: text(album, "id"),
                kind: RowKind::Album,
                title: text(album, "title"),
                subtitle: text(album, "artist"),
                meta,
                cover_url: text(album, "cover"),
                song_id: String::new(),
                album_id: text(album, "id"),
                starred: album["starred"].as_bool().unwrap_or(false),
                rating: album["rating"].as_i64().unwrap_or(0),
                downloaded: false,
            });
        }
    }
    if let Some(songs) = value["songs"].as_array() {
        for song in songs.iter().take(limit) {
            let plays = song["play_count"].as_i64().unwrap_or(0);
            rows.push(Row {
                track: song["track"]
                    .as_i64()
                    .map(|track| track.to_string())
                    .unwrap_or_default(),
                id: text(song, "id"),
                kind: RowKind::Song,
                title: text(song, "title"),
                subtitle: text(song, "subtitle"),
                meta: if most_played {
                    if plays == 1 {
                        "1 play".to_string()
                    } else {
                        format!("{plays} plays")
                    }
                } else {
                    text(song, "meta")
                },
                cover_url: text(song, "cover"),
                song_id: text(song, "songId"),
                album_id: text(song, "albumId"),
                starred: song["starred"].as_bool().unwrap_or(false),
                rating: song["rating"].as_i64().unwrap_or(0),
                downloaded: false,
            });
        }
    }
    if let Some(playlists) = value["playlists"].as_array() {
        for playlist in playlists.iter().take(limit) {
            rows.push(Row {
                track: String::new(),
                id: text(playlist, "id"),
                kind: RowKind::Playlist,
                title: playlist["name"]
                    .as_str()
                    .or_else(|| playlist["title"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                subtitle: text(playlist, "subtitle"),
                meta: text(playlist, "meta"),
                cover_url: text(playlist, "cover"),
                song_id: String::new(),
                album_id: String::new(),
                starred: false,
                rating: 0,
                downloaded: false,
            });
        }
    }
    Ok((rows, most_played))
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

fn unix_ms_for_cover() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn send_event(tx: &UnboundedSender<Event>, event: Event) {
    let _ = tx.send(event);
}

fn thumbify(url: String) -> String {
    if url.is_empty() || url.contains(".thumb.") || url.contains("#no-thumb") || !url.contains(".jpg") {
        return url;
    }
    url.replacen(".jpg", ".thumb.jpg", 1)
}

fn plain_notes(raw: &str) -> String {
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
