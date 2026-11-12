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
                Ok(()) => PlaylistSave::ok(name).created(offline).into(),
                Err(err) => PlaylistSave::failed(name, format!("{err:#}"))
                    .created(offline)
                    .into(),
            };
            let _ = tx.send(event);
        });
    }
}

impl Bridge {
    /// Resolve and run a scoped download/removal request off the UI thread.
    pub fn download_request(
        &self,
        request: crate::ipc_command::DownloadRequest,
        current: Option<crate::ipc_command::CurrentTrack>,
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
                    let _ = tx.send(PlaylistSave::ok(name).into());
                }
                Err(err) => {
                    let _ = tx.send(PlaylistSave::failed(playlist, format!("{err:#}")).into());
                }
            }
        });
    }
}

impl Bridge {
    /// Append the UI's current selection to an existing playlist
    /// (`playlist add selected; <playlist>`). The selection lives in the UI
    /// process, so the UI sends one row kind plus the selected ids; album and
    /// artist selections expand here, like the row-clipboard paste.
    pub fn playlist_add_selected(&self, kind: String, ids: Vec<String>, playlist: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let event = match crate::playlists::add_selection_to_playlist(
                &dbs, &kind, &ids, &playlist,
            )
            .await
            {
                Ok((name, rows)) => PlaylistSave::ok(name).rows(rows).into(),
                Err(err) => PlaylistSave::failed(playlist, format!("{err:#}")).into(),
            };
            let _ = tx.send(event);
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
                    let _ = tx.send(PlaylistSave::ok(name).removed().into());
                }
                Err(err) => {
                    let _ = tx.send(
                        PlaylistSave::failed(playlist, format!("{err:#}"))
                            .removed()
                            .into(),
                    );
                }
            }
        });
    }
}

impl Bridge {
    /// Delete a whole playlist (local M3U file or server playlist).
    pub fn delete_playlist(&self, playlist: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::delete_playlist(&dbs, &playlist).await {
                Ok((id, name)) => {
                    let _ = tx.send(Event::PlaylistDeleted {
                        id,
                        name,
                        error: None,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::PlaylistDeleted {
                        id: String::new(),
                        name: playlist,
                        error: Some(format!("{err:#}")),
                    });
                }
            }
        });
    }
}

impl Bridge {
    /// Paste the row clipboard into a playlist page at `position` (the row
    /// below/above the cursor). Resolves every clipboard id to its songs
    /// (album/artist expand), then inserts the song ids.
    pub fn paste_playlist(
        &self,
        playlist_id: String,
        kind: String,
        ids: Vec<String>,
        position: usize,
    ) {
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
            let rows = song_ids.len();
            match crate::playlists::insert_song_ids_at(&dbs, &playlist_id, position, &song_ids).await {
                Ok(name) => {
                    let _ = tx.send(
                        PlaylistSave::ok(name)
                            .rows(rows)
                            .at(playlist_id, Some(position))
                            .into(),
                    );
                }
                Err(err) => {
                    let _ = tx.send(PlaylistSave::failed(playlist_id, format!("{err:#}")).into());
                }
            }
        });
    }

    /// Move one playlist entry (the page `Ctrl-Shift-J/K` path).
    pub fn move_playlist_row(&self, playlist_id: String, from: usize, to: usize) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::move_playlist_entry(&dbs, &playlist_id, from, to).await {
                Ok(name) => {
                    let _ = tx.send(
                        PlaylistSave::ok(name)
                            .moved()
                            .at(playlist_id, Some(to))
                            .into(),
                    );
                }
                Err(err) => {
                    let _ = tx.send(PlaylistSave::failed(playlist_id, format!("{err:#}")).into());
                }
            }
        });
    }

    /// Move a block of playlist rows (a visual selection reorder). `cursor` is
    /// the reordered cursor position to reveal after the reload.
    pub fn move_playlist_block(
        &self,
        playlist_id: String,
        start: usize,
        len: usize,
        delta: i32,
        cursor: usize,
    ) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::move_playlist_block(&dbs, &playlist_id, start, len, delta).await {
                Ok(name) => {
                    let _ = tx.send(
                        PlaylistSave::ok(name)
                            .moved()
                            .at(playlist_id, Some(cursor))
                            .into(),
                    );
                }
                Err(err) => {
                    let _ = tx.send(PlaylistSave::failed(playlist_id, format!("{err:#}")).into());
                }
            }
        });
    }

    /// Delete playlist entries by position (the playlist page `d d` path).
    pub fn delete_playlist_positions(&self, playlist_id: String, positions: Vec<usize>) {
        let tx = self.tx.clone();
        let rows = positions.len();
        let cursor = positions.first().copied();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::playlists::remove_playlist_positions(&dbs, &playlist_id, &positions).await {
                Ok(name) => {
                    let _ = tx.send(
                        PlaylistSave::ok(name)
                            .removed()
                            .rows(rows)
                            .at(playlist_id, cursor)
                            .into(),
                    );
                }
                Err(err) => {
                    let _ = tx.send(
                        PlaylistSave::failed(playlist_id, format!("{err:#}"))
                            .removed()
                            .into(),
                    );
                }
            }
        });
    }
}
