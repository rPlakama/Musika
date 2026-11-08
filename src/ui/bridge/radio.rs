//! Radio playback and Radio Tool requests.

use super::*;

impl Bridge {
    pub fn quick_radio(&self, kind: String, id: String) {
        self.play_radio(kind, id, 0);
    }

    pub fn play_radio(&self, kind: String, id: String, start_index: usize) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue_pair(
                &tx,
                crate::radio::tracks_json(&dbs, &kind, &id, start_index as i64)
                    .await
                    .map(|(json, start)| (json, start as usize)),
                "Could not start radio",
            );
        });
    }

    /// Radio Tool: save the generated rows locally and post them as a
    /// Subsonic/Navidrome playlist. A local-only success is still reported.
    pub fn save_radio(&self, seed_kind: String, seed_id: String, name: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::radio::save_radio(&dbs, &seed_kind, &seed_id, &name).await {
                Ok(saved) => {
                    let _ = tx.send(Event::RadioSaved {
                        name: saved.name,
                        offline: false,
                        remote_error: saved.remote_error,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not save radio: {err:#}")));
                }
            }
        });
    }

    /// Radio Tool: download every generated row. Best-effort per song, like
    /// the artist-container download.
    pub fn download_radio(&self, seed_kind: String, seed_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let rows = match crate::radio::radio_rows(&dbs, &seed_kind, &seed_id).await {
                Ok(rows) => rows,
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not load radio: {err:#}")));
                    return;
                }
            };
            if rows.is_empty() {
                let _ = tx.send(Event::Error(
                    "This radio has no songs to download.".to_string(),
                ));
                return;
            }
            for song in rows {
                if let Err(err) = crate::media_actions::download_one(&dbs, &song.id).await {
                    let _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                }
            }
            let _ = tx.send(Event::DownloadSettled);
        });
    }

    /// Radio Tool: replace the queue with a shuffled copy of the generated
    /// rows.
    pub fn shuffle_radio(&self, seed_kind: String, seed_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue(
                &tx,
                crate::radio::shuffled_tracks_json(&dbs, &seed_kind, &seed_id, offline).await,
                0,
                "Could not shuffle the radio",
            );
        });
    }

    /// Radio Tool: append the whole generated queue without replacing what is
    /// already playing.
    pub fn queue_add_radio(&self, seed_kind: String, seed_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let json = match crate::radio::tracks_json(&dbs, &seed_kind, &seed_id, 0).await {
                Ok((json, _)) => json,
                Err(err) => {
                    let _ = tx.send(Event::Error(format!(
                        "Could not add the radio to the queue: {err:#}"
                    )));
                    return;
                }
            };
            emit_append(&tx, Ok(json), "Could not add the radio to the queue");
        });
    }

    pub fn quick_go_album(&self, song_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(song)) = crate::library::song(&dbs.library, &song_id).await else {
                return;
            };
            let Some(album_id) = song.album_id else {
                return;
            };
            let _ = tx.send(Event::OpenPage {
                descriptor: super::super::state::PageDescriptor::new(
                    super::super::state::PageKind::Album,
                    album_id,
                ),
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
                    super::super::state::PageDescriptor::new(
                        super::super::state::PageKind::Artist,
                        artist_id,
                    ),
                    super::super::state::RowKind::Album,
                ),
                // No artist mapping: keep the album reachable.
                None => (
                    super::super::state::PageDescriptor::new(
                        super::super::state::PageKind::Album,
                        album_id.clone(),
                    ),
                    super::super::state::RowKind::Album,
                ),
            };
            let _ = tx.send(Event::OpenPageReveal {
                descriptor,
                row_kind,
                row_id: album_id,
            });
        });
    }
}

impl Bridge {
    /// `radio save --offline`: write only the app-owned local M3U.
    pub fn save_radio_local(&self, seed_kind: String, seed_id: String, name: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::radio::save_radio_local(&dbs, &seed_kind, &seed_id, &name).await {
                Ok(saved) => {
                    let _ = tx.send(Event::RadioSaved {
                        name: saved.name,
                        offline: true,
                        remote_error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not save radio: {err:#}")));
                }
            }
        });
    }
}

impl Bridge {
    /// Resolve an album/song name through the search engine, then start radio.
    pub fn play_radio_query(&self, kind: String, query: String, start_index: usize) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let scope = if kind == "album" {
                crate::ui::ipc_command::DownloadScope::Album
            } else {
                crate::ui::ipc_command::DownloadScope::Track
            };
            let result = async {
                let id = crate::download_command::resolve_named_id(&dbs, scope, &query).await?;
                let _ = tx.send(Event::OpenPage {
                    descriptor: super::super::state::PageDescriptor::radio(
                        kind.clone(),
                        id.clone(),
                    ),
                });
                crate::radio::tracks_json(&dbs, &kind, &id, start_index as i64)
                    .await
                    .map(|(json, start)| (json, start as usize))
            }
            .await;
            emit_queue_pair(&tx, result, "Could not start radio");
        });
    }
}
