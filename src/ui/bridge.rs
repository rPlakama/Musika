// Musika — GPUI backend bridge (G1).
//!
//! UI thread never touches tokio/DBs directly. Requests go to the shared
//! runtime and return as plain `Event`s over a tokio channel; the GPUI root
//! task drains them and notifies its entity.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc::UnboundedSender;

pub enum Event {
    Library {
        generation: u64,
        scope: super::state::Scope,
        order: usize,
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
}

impl Bridge {
    pub fn new(tx: UnboundedSender<Event>) -> Self {
        let bridge = Self {
            tx,
            generation: AtomicU64::new(0),
        };
        bridge.load_header();
        bridge
    }

    fn load_header(&self) {
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
                    Ok(None) => Event::Error("No saved account yet".to_string()),
                    Err(err) => Event::Error(format!("Could not read credentials: {err:#}")),
                },
                None => Event::Error("The library is not initialised".to_string()),
            };
            let _ = tx.send(event);
        });
    }

    pub fn load_library(&self, scope: super::state::Scope, order: usize) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
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
                })
                .collect())
        }
    }
}

fn songs_label(count: i64) -> String {
    if count == 1 { "1 song".to_string() } else { format!("{count} songs") }
}

