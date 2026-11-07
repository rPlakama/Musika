// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
                            server_type: credentials.server_type,
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
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let event = match page_data(&descriptor, offline).await {
                Ok((header, rows)) => {
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

    pub fn request_notes(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let cached = crate::library::album_info(&dbs.library, &album_id)
                .await
                .ok()
                .flatten();
            if let Some((notes, fetched_at)) = &cached {
                let _ = tx.send(Event::Notes {
                    album_id: album_id.clone(),
                    notes: plain_notes(notes),
                });
                if crate::consts::now_secs() - fetched_at < crate::consts::INFO_TTL_SECS {
                    return;
                }
            }
            let fresh = match crate::session::client(&dbs).await {
                Ok(client) => client
                    .get_album_info2(&album_id)
                    .await
                    .ok()
                    .and_then(|info| info.notes)
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            if !fresh.is_empty()
                && let Err(err) =
                    crate::library::upsert_album_info(&dbs.library, &album_id, &fresh).await
            {
                crate::debug_log!("GPUI: album notes cache failed: {err:#}");
            }
            let notes = if fresh.is_empty() {
                cached.map(|(notes, _)| notes).unwrap_or_default()
            } else {
                fresh
            };
            let _ = tx.send(Event::Notes {
                album_id,
                notes: plain_notes(&notes),
            });
        });
    }

    pub fn request_artist_notes(&self, artist_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let cached = crate::library::artist_info(&dbs.library, &artist_id)
                .await
                .ok()
                .flatten();
            if let Some(row) = &cached {
                let _ = tx.send(Event::Notes {
                    album_id: artist_id.clone(),
                    notes: plain_notes(&row.biography),
                });
                if row
                    .fetched_at
                    .parse::<i64>()
                    .is_ok_and(|at| crate::consts::now_secs() - at < crate::consts::INFO_TTL_SECS)
                {
                    return;
                }
            }
            let fresh = match crate::session::client(&dbs).await {
                Ok(client) => match client.get_artist_info2(&artist_id, Some(10), None).await {
                    Ok(info) => info.info.biography.unwrap_or_default(),
                    Err(_) => client
                        .get_artist_info(&artist_id, Some(10), None)
                        .await
                        .ok()
                        .and_then(|info| info.info.biography)
                        .unwrap_or_default(),
                },
                Err(_) => String::new(),
            };
            if !fresh.is_empty() {
                let _ = crate::library::upsert_artist_info(&dbs.library, &artist_id, &fresh, "").await;
            }
            let notes = if fresh.is_empty() {
                cached.map(|row| row.biography).unwrap_or_default()
            } else {
                fresh
            };
            let _ = tx.send(Event::Notes {
                album_id: artist_id,
                notes: plain_notes(&notes),
            });
        });
    }

    pub fn restore(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(status)) = crate::status::load(&dbs.status).await else {
                return;
            };
            let resume = crate::queue::resume_payload(&dbs, &status, false).await.ok();
            let value: serde_json::Value = resume
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or(serde_json::Value::Null);
            let tracks = value["queue"]
                .as_str()
                .and_then(|json| serde_json::from_str::<Vec<crate::player::Track>>(json).ok())
                .unwrap_or_default();
            let start = value["start"].as_u64().unwrap_or(0) as usize;
            let position = value["position"].as_f64().unwrap_or(status.position);
            crate::debug_log!(
                "GPUI: restore {} tracks start {} position {:.1}",
                tracks.len(),
                start,
                position
            );
            let _ = tx.send(Event::Restore {
                volume: status.volume,
                muted: status.muted,
                tracks,
                start,
                position,
                page_kind: status.page_kind,
                page_id: status.page_id,
            });
        });
    }

    pub fn request_lyrics(&self, song_id: String) {
        crate::debug_log!("GPUI: lyrics request {song_id}");
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let client = crate::session::client(&dbs).await.ok();
            let document = match crate::lyrics::fetch(&dbs, client.as_ref(), &song_id).await {
                Ok(document) => document,
                Err(err) => {
                    crate::debug_log!("GPUI: lyrics fetch failed for {song_id}: {err:#}");
                    crate::lyrics::Document::missing(&song_id)
                }
            };
            let _ = tx.send(Event::Lyrics { song_id, document });
        });
    }

    pub fn play_album(&self, album_id: String, start: usize) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::queue::album_tracks_json(&dbs, &album_id, offline).await {
                Ok(json) => match serde_json::from_str::<Vec<crate::player::Track>>(&json) {
                    Ok(tracks) => {
                        let _ = tx.send(Event::Queue {
                            tracks,
                            start,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(Event::Error(format!("Could not play album: {err:#}")));
                    }
                },
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not play album: {err:#}")));
                }
            }
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
        self.set_offline(false);
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
        self.set_offline(true);
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
        let generation_counter = self.search_generation.clone();
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            // Debounce keystrokes: only the latest query runs the index search.
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            if generation_counter.load(Ordering::Relaxed) != generation {
                return;
            }
            let event = match search_rows(&query, offline, super::state::SEARCH_FETCH_ROWS).await
            {
                Ok((rows, most_played)) => {
                    crate::debug_log!("GPUI: search {:?} {} rows", query, rows.len());
                    Event::Search {
                        generation,
                        query,
                        rows,
                        most_played,
                    }
                }
                Err(err) => Event::Error(format!("Search failed: {err:#}")),
            };
            let _ = tx.send(event);
        });
    }

    pub fn load_library(&self, scope: super::state::Scope, order: usize) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        if scope == super::state::Scope::Playlists {
            crate::app::runtime().spawn(async move {
                let event = match library_rows(scope, order).await {
                    Ok(rows) => {
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
            return generation;
        }
        crate::app::runtime().spawn(async move {
            let event = match crate::app::dbs() {
                Some(dbs) => {
                    let surface = scope.key();
                    match crate::rows::count(
                        &dbs,
                        surface,
                        "",
                        &order_name(scope, order),
                        offline,
                    )
                    .await
                    {
                        Ok(count) => Event::LibraryCount {
                            generation,
                            scope,
                            order,
                            count,
                        },
                        Err(err) => Event::Error(format!("Library count failed: {err:#}")),
                    }
                }
                None => Event::Error("databases not ready".to_string()),
            };
            let _ = tx.send(event);
        });
        generation
    }

    pub fn request_library_window(
        &self,
        generation: u64,
        scope: super::state::Scope,
        order: usize,
        start: usize,
        count: usize,
    ) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let surface = scope.key();
            match crate::rows::window(
                &dbs,
                surface,
                "",
                &order_name(scope, order),
                start,
                count,
                offline,
            )
            .await
            {
                Ok(value) => {
                    let rows = value["rows"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| row_from_json(scope, row))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(Event::LibraryWindow {
                        generation,
                        scope,
                        order,
                        start,
                        rows,
                    });
                }
                Err(err) => {
                    crate::debug_log!("GPUI: row window {surface} failed: {err:#}");
                }
            }
        });
    }

    pub fn quick_favorite(&self, kind: String, id: String, on: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = async {
                let client = crate::session::client(&dbs).await?;
                if on {
                    match kind.as_str() {
                        "song" => client.star(&[id.as_str()], &[], &[]).await,
                        "album" => client.star(&[], &[id.as_str()], &[]).await,
                        "artist" => client.star(&[], &[], &[id.as_str()]).await,
                        other => anyhow::bail!("unknown favourite kind: {other}"),
                    }
                } else {
                    match kind.as_str() {
                        "song" => client.unstar(&[id.as_str()], &[], &[]).await,
                        "album" => client.unstar(&[], &[id.as_str()], &[]).await,
                        "artist" => client.unstar(&[], &[], &[id.as_str()]).await,
                        other => anyhow::bail!("unknown favourite kind: {other}"),
                    }
                }
            };
            if let Err(err) = result.await {
                let _ = tx.send(Event::Error(format!("Could not update the favourite: {err:#}")));
                return;
            }
            let local = match kind.as_str() {
                "song" => crate::library::set_song_starred(&dbs.library, &id, on).await,
                "album" => crate::library::set_album_starred(&dbs.library, &id, on).await,
                _ => crate::artist_user::set_starred(&dbs.artist_user, &id, on).await,
            };
            if let Err(err) = local {
                let _ = tx.send(Event::Error(format!("Could not save the favourite: {err:#}")));
                return;
            }
            let row_kind = match kind.as_str() {
                "song" => super::state::RowKind::Song,
                "album" => super::state::RowKind::Album,
                _ => super::state::RowKind::Artist,
            };
            let _ = tx.send(Event::RowsPatched {
                kind: row_kind,
                id,
                starred: Some(on),
                rating: None,
            });
        });
    }

    pub fn quick_rate(&self, kind: String, id: String, level: u8) {
        let level = level.min(5);
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = async {
                let client = crate::session::client(&dbs).await?;
                client.set_rating(&id, level).await?;
                match kind.as_str() {
                    "artist" => crate::artist_user::set_rating(&dbs.artist_user, &id, level).await,
                    _ => crate::library::set_album_rating(&dbs.library, &id, level).await,
                }
            }
            .await;
            if let Err(err) = result {
                let _ = tx.send(Event::Error(format!("Could not rate: {err:#}")));
                return;
            }
            let row_kind = if kind == "artist" {
                super::state::RowKind::Artist
            } else {
                super::state::RowKind::Album
            };
            let _ = tx.send(Event::RowsPatched {
                kind: row_kind,
                id,
                starred: None,
                rating: Some(level),
            });
        });
    }

    pub fn quick_download(&self, kind: String, id: String, on: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            if !on {
                let result = if kind == "album" {
                    crate::media_actions::album_container_off(&dbs, &id).await;
                    Ok(())
                } else if kind == "song" {
                    let unmark = crate::download::unmark(&dbs.download, &id).await;
                    let media = crate::media::remove_songs_media(&dbs.media, std::slice::from_ref(&id)).await;
                    match (unmark, media) {
                        (Err(err), _) | (_, Err(err)) => Err(err),
                        _ => Ok(()),
                    }
                } else {
                    Ok(())
                };
                if let Err(err) = result {
                    let _ = tx.send(Event::Error(format!("Could not remove download: {err:#}")));
                } else {
                    let _ = tx.send(Event::DownloadSettled);
                }
                return;
            }
            let songs = if kind == "song" {
                vec![id.clone()]
            } else if kind == "album" {
                crate::library::album_song_ids_of(&dbs.library, std::slice::from_ref(&id))
                    .await
                    .ok()
                    .and_then(|mut map| map.remove(&id))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if songs.is_empty() {
                let _ = tx.send(Event::Error("No songs to download for this item".to_string()));
                return;
            }
            for song in songs {
                if let Err(err) = crate::media_actions::download_one(&dbs, &song).await {
                    let _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                }
            }
            let _ = tx.send(Event::DownloadSettled);
        });
    }

    pub fn quick_queue_add(&self, kind: String, id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let queued = if kind == "playlist" {
                crate::queue::playlist_tracks_json(&dbs, &id, offline, 0)
                    .await
                    .map(|(json, _)| json)
            } else {
                crate::queue::quick_tracks_json(&dbs, &kind, &id, offline).await
            };
            match queued {
                Ok(json) => match serde_json::from_str::<Vec<crate::player::Track>>(&json) {
                    Ok(tracks) => {
                        let _ = tx.send(Event::QueueAppend { tracks });
                    }
                    Err(err) => {
                        let _ = tx.send(Event::Error(format!("Could not add to queue: {err:#}")));
                    }
                },
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not add to queue: {err:#}")));
                }
            }
        });
    }

    pub fn quick_radio(&self, kind: String, id: String) {
        self.play_radio(kind, id, 0);
    }

    pub fn play_radio(&self, kind: String, id: String, start_index: usize) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::radio::tracks_json(&dbs, &kind, &id, start_index as i64).await {
                Ok((json, start)) => match serde_json::from_str::<Vec<crate::player::Track>>(&json) {
                    Ok(tracks) => {
                        let _ = tx.send(Event::Queue {
                            tracks,
                            start: start as usize,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(Event::Error(format!("Could not start radio: {err:#}")));
                    }
                },
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not start radio: {err:#}")));
                }
            }
        });
    }

    pub fn quick_go_album(&self, song_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(song)) = crate::library::song(&dbs.library, &song_id).await else {
                return;
            };
            let Some(album_id) = song.album_id else { return };
            let _ = tx.send(Event::OpenPage {
                descriptor: super::state::PageDescriptor::new(super::state::PageKind::Album, album_id),
            });
        });
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

    pub fn quick_go_artist(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(artist_id)) = crate::library::album_artist_id(&dbs.library, &album_id).await
            else {
                return;
            };
            let _ = tx.send(Event::OpenPage {
                descriptor: super::state::PageDescriptor::new(super::state::PageKind::Artist, artist_id),
            });
        });
    }

    /// Search activation: open the album's artist page and reveal the album row.
    pub fn open_album_artist_reveal(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let artist_id = crate::library::album_artist_id(&dbs.library, &album_id)
                .await
                .ok()
                .flatten();
            let (descriptor, row_kind) = match artist_id {
                Some(artist_id) => (
                    super::state::PageDescriptor::new(
                        super::state::PageKind::Artist,
                        artist_id,
                    ),
                    super::state::RowKind::Album,
                ),
                // No artist mapping: keep the album reachable.
                None => (
                    super::state::PageDescriptor::new(
                        super::state::PageKind::Album,
                        album_id.clone(),
                    ),
                    super::state::RowKind::Album,
                ),
            };
            let _ = tx.send(Event::OpenPageReveal {
                descriptor,
                row_kind,
                row_id: album_id,
            });
        });
    }

    pub fn quick_play_playlist(&self, id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::queue::playlist_tracks_json(&dbs, &id, offline, 0).await {
                Ok((json, start)) => match serde_json::from_str::<Vec<crate::player::Track>>(&json) {
                    Ok(tracks) => {
                        let _ = tx.send(Event::Queue {
                            tracks,
                            start: start as usize,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(Event::Error(format!("Could not play the playlist: {err:#}")));
                    }
                },
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not play the playlist: {err:#}")));
                }
            }
        });
    }

    pub fn request_colors(&self, cover_art: String) {
        if cover_art.is_empty() {
            return;
        }
        {
            let mut inflight = self
                .palette_inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inflight.as_deref() == Some(cover_art.as_str()) {
                return;
            }
            *inflight = Some(cover_art.clone());
        }
        let tx = self.tx.clone();
        let inflight_shared = Arc::clone(&self.palette_inflight);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let event = match crate::colors::fetch(&dbs, &cover_art).await {
                Ok(Some(scheme)) => Event::Palette {
                    cover_art: cover_art.clone(),
                    roles: scheme.roles,
                },
                _ => Event::PaletteReset {
                    cover_art: cover_art.clone(),
                },
            };
            let _ = tx.send(event);
            let mut inflight = inflight_shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if inflight.as_deref() == Some(cover_art.as_str()) {
                *inflight = None;
            }
        });
    }

    pub fn request_display_cover(&self, key: String, cover_art: String, target: u32) {
        if cover_art.is_empty() || target == 0 {
            return;
        }
        let inflight = Arc::clone(&self.display_inflight);
        {
            let mut keys = inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !keys.insert(key.clone()) {
                return;
            }
        }
        let tx = self.tx.clone();
        let started = std::time::Instant::now();
        crate::debug_log!(
            "covers: display request cover={} target={} key={} at={}",
            cover_art,
            target,
            key,
            unix_ms_for_cover()
        );
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else {
                inflight
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&key);
                return;
            };
            let source = match crate::covers::cached_path(&dbs.covers, &cover_art).await {
                Ok(Some(source)) => source,
                _ => {
                    crate::debug_log!(
                        "covers: display failed cover={} target={} reason=no-source key={}",
                        cover_art,
                        target,
                        key
                    );
                    let _ = tx.send(Event::DisplayCover {
                        key: key.clone(),
                        cover_art: cover_art.clone(),
                        target,
                        path: None,
                    });
                    inflight
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&key);
                    return;
                }
            };
            let path = crate::covers::get_or_create_resized_cover(&source, target).await;
            crate::debug_log!(
                "covers: display ready cover={} target={} path={:?} elapsed_ms={:.1}",
                cover_art,
                target,
                path,
                started.elapsed().as_secs_f64() * 1000.0
            );
            let _ = tx.send(Event::DisplayCover {
                key: key.clone(),
                cover_art: cover_art.clone(),
                target,
                path,
            });
            inflight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        });
    }

    pub fn preload_library(&self, scope: super::state::Scope, order: usize) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(count) = crate::rows::count(
                &dbs,
                scope.key(),
                "",
                &order_name(scope, order),
                offline,
            )
            .await
            else {
                return;
            };
            if count == 0 {
                return;
            }
            let Ok(value) = crate::rows::window(
                &dbs,
                scope.key(),
                "",
                &order_name(scope, order),
                0,
                50,
                offline,
            )
            .await
            else {
                return;
            };
            let rows = value["rows"]
                .as_array()
                .map(|rows| rows.iter().map(|row| row_from_json(scope, row)).collect())
                .unwrap_or_default();
            let _ = tx.send(Event::Preload {
                scope,
                order,
                count,
                rows,
            });
        });
    }

    pub fn request_marks(&self, songs: Vec<String>, albums: Vec<String>) {
        let generation = self.marks_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::download::window_state(&dbs, &songs, &albums).await {
                Ok(result) => {
                    let _ = tx.send(Event::Marks { generation, result });
                }
                Err(err) => {
                    crate::debug_log!("GPUI: mark window failed: {err:#}");
                }
            }
        });
    }

    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::Relaxed);
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
                subtitle: format!("{} albums", artist.album_count),
                meta: String::new(),
                cover_url: thumbify(crate::covers::url_for(artist.cover_art.as_deref(), &covers)),
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
