//! Playlist creation from the current selection.

use super::*;

impl Bridge {
    /// Remote playlists are posted to the server; `--offline` writes only the
    /// account-local M3U. A remote failure never creates a local fallback.
    pub fn create_playlist(&self, name: String, song_ids: Vec<String>, offline: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else {
                return;
            };
            let result = if offline {
                crate::playlists::create_local(&dbs, &name, &song_ids)
                    .await
                    .map(|_| ())
            } else {
                crate::playlists::create_remote(&dbs, &name, &song_ids).await
            };
            let event = match result {
                Ok(()) => Event::PlaylistSaved {
                    name,
                    offline,
                    created: true,
                    removed: false,
                    rows: 0,
                    error: None,
                },
                Err(err) => Event::PlaylistSaved {
                    name,
                    offline,
                    created: true,
                    removed: false,
                    rows: 0,
                    error: Some(format!("{err:#}")),
                },
            };
            let _ = tx.send(event);
        });
    }
}

impl Bridge {
    /// Resolve and run a scoped download/removal request off the UI thread.
    pub fn download_request(
        &self,
        request: crate::ui::ipc_command::DownloadRequest,
        current: Option<crate::ui::ipc_command::CurrentTrack>,
    ) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::download_command::execute(&dbs, &request, current.as_ref()).await {
                Ok(()) => {
                    let _ = tx.send(Event::DownloadSettled);
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Download command failed: {err:#}")));
                }
            }
        });
    }
}

impl Bridge {
    /// Append targets to an existing local/server playlist. Target phrases are
    /// resolved by the playlist backend, which mirrors the download search.
    pub fn playlist_add(
        &self,
        targets: Vec<String>,
        playlist: String,
        current_song_id: Option<String>,
    ) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::add_to_playlist(
                &dbs,
                &targets,
                &playlist,
                current_song_id.as_deref(),
            )
            .await
            {
                Ok(name) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name,
                        offline: false,
                        created: false,
                        removed: false,
                        rows: 0,
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name: playlist,
                        offline: false,
                        created: false,
                        removed: false,
                        rows: 0,
                        error: Some(format!("{err:#}")),
                    });
                }
            }
        });
    }
}

impl Bridge {
    /// Remove targets from an existing local/server playlist, mirroring
    /// [`Self::playlist_add`]; targets resolve through the same playlists
    /// backend (`playlist rm <target...>; <playlist>`).
    pub fn playlist_remove(
        &self,
        targets: Vec<String>,
        playlist: String,
        current_song_id: Option<String>,
    ) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::remove_from_playlist(
                &dbs,
                &targets,
                &playlist,
                current_song_id.as_deref(),
            )
            .await
            {
                Ok(name) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name,
                        offline: false,
                        created: false,
                        removed: true,
                        rows: 0,
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name: playlist,
                        offline: false,
                        created: false,
                        removed: true,
                        rows: 0,
                        error: Some(format!("{err:#}")),
                    });
                }
            }
        });
    }
}

impl Bridge {
    /// Paste the row clipboard into a playlist page: resolve every clipboard id
    /// to its songs (album/artist expand), then append the song ids. Empty
    /// resolution reports an error instead of silently doing nothing.
    pub fn paste_playlist(&self, playlist_id: String, kind: String, ids: Vec<String>) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let songs = match crate::queue::quick_songs_many(&dbs, &kind, &ids).await {
                Ok(songs) => songs,
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Could not paste: {err:#}")));
                    return;
                }
            };
            let song_ids: Vec<String> = songs.iter().map(|song| song.id.clone()).collect();
            if song_ids.is_empty() {
                let _ = tx.send(Event::Error(
                    "Could not paste: the yanked rows no longer resolve".to_string(),
                ));
                return;
            }
            match crate::playlists::add_song_ids_to_playlist(&dbs, &playlist_id, &song_ids).await {
                Ok(name) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name,
                        offline: false,
                        created: false,
                        removed: false,
                        rows: song_ids.len(),
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name: playlist_id,
                        offline: false,
                        created: false,
                        removed: false,
                        rows: 0,
                        error: Some(format!("{err:#}")),
                    });
                }
            }
        });
    }

    /// Delete playlist entries by position (the playlist page `d d` path).
    pub fn delete_playlist_positions(&self, playlist_id: String, positions: Vec<usize>) {
        let tx = self.tx.clone();
        let rows = positions.len();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::remove_playlist_positions(&dbs, &playlist_id, &positions).await {
                Ok(name) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name,
                        offline: false,
                        created: false,
                        removed: true,
                        rows,
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::PlaylistSaved {
                        name: playlist_id,
                        offline: false,
                        created: false,
                        removed: true,
                        rows: 0,
                        error: Some(format!("{err:#}")),
                    });
                }
            }
        });
    }
}
