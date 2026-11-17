//! Sync, wipe and container-maintenance requests.

use super::*;

impl Bridge {
    /// Run an incremental, deep, or clean-rebuild library sync.
    pub fn sync(&self, mode: SyncMode) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(client) = crate::session::client(&dbs).await else {
                _ = tx.send(Event::Error("Sync needs a live session.".to_string()));
                return;
            };
            let client = Arc::new(client);
            if mode == SyncMode::Repair {
                _ = tx.send(Event::SyncProgress {
                    message: "Repairing library…".to_string(),
                    done: 0,
                    total: 100,
                });
                if let Err(err) = crate::db::wipe_library(&dbs).await {
                    _ = tx.send(Event::Error(format!("Library repair failed: {err:#}")));
                    return;
                }
            }
            let deep = mode != SyncMode::Incremental;
            let progress_tx = tx.clone();
            let report = match crate::index::sync_library(&dbs, client.clone(), deep, |progress| {
                _ = progress_tx.send(Event::SyncProgress {
                    message: progress.message,
                    done: progress.done,
                    total: progress.total,
                });
            })
            .await
            {
                Ok(report) => report,
                Err(err) => {
                    _ = tx.send(Event::Error(format!("Library sync failed: {err:#}")));
                    crate::linux::malloc_trim();
                    return;
                }
            };
            if mode != SyncMode::Incremental && report.covers_due {
                _ = tx.send(Event::SyncProgress {
                    message: "Refreshing covers…".to_string(),
                    done: 90,
                    total: 100,
                });
                crate::index::cache_covers(&dbs, &client, &report.forced_covers).await;
            }
            // A sync holds large JSON bodies and cover buffers transiently;
            // return the freed pages now instead of keeping the high-water
            // mark resident for the daemon's whole lifetime.
            crate::linux::malloc_trim();
            _ = tx.send(Event::LibrarySynced);
        });
    }

    /// Wipe a maintenance scope. Library/index/cover scopes are independent.
    pub fn wipe(&self, scope: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = match scope.as_str() {
                "library" => crate::db::wipe_library(&dbs).await,
                "indexes" => crate::db::wipe_indexes(&dbs).await,
                "covers" => crate::db::wipe_covers(&dbs).await,
                "lyrics" => crate::db::wipe_lyrics(&dbs).await,
                "album" => crate::media_actions::wipe_albums(&dbs).await,
                other => Err(anyhow::anyhow!("unknown wipe scope: {other}")),
            };
            match result {
                Ok(()) => {
                    _ = tx.send(Event::Wiped { scope });
                }
                Err(err) => {
                    _ = tx.send(Event::Error(format!("Wipe failed: {err:#}")));
                }
            }
            crate::linux::malloc_trim();
        });
    }

    /// Download coverage + favourite/rating for an Album or Artist page.
    pub fn request_container_state(&self, kind: crate::model::PageKind, id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let (cached, starred, rating) = match kind {
                crate::model::PageKind::Album => {
                    let cached =
                        crate::download::window_state(&dbs, &[], std::slice::from_ref(&id))
                            .await
                            .map(|marks| marks.albums_done.iter().any(|album| album == &id))
                            .unwrap_or(false);
                    let state =
                        crate::library::album_states(&dbs.library, std::slice::from_ref(&id))
                            .await
                            .ok()
                            .and_then(|states| states.get(&id).copied())
                            .unwrap_or((false, 0));
                    (cached, state.0, state.1.clamp(0, 5) as u8)
                }
                crate::model::PageKind::Artist => {
                    let albums = crate::library::artist_album_ids(&dbs.library, &id)
                        .await
                        .unwrap_or_default();
                    let cached = if albums.is_empty() {
                        false
                    } else {
                        crate::download::window_state(&dbs, &[], &albums)
                            .await
                            .map(|marks| marks.albums_done.len() == albums.len())
                            .unwrap_or(false)
                    };
                    let user = crate::artist_user::get(&dbs.artist_user, &id)
                        .await
                        .ok()
                        .flatten();
                    let (starred, rating) = user
                        .map(|user| (user.starred, user.rating.clamp(0, 5) as u8))
                        .unwrap_or((false, 0));
                    (cached, starred, rating)
                }
                crate::model::PageKind::Playlist | crate::model::PageKind::Radio => {
                    (false, false, 0)
                }
            };
            _ = tx.send(Event::PageContainerState {
                kind,
                id,
                cached,
                starred,
                rating,
            });
        });
    }

    /// Download or remove the whole artist container.
    pub fn download_artist(&self, artist_id: String, on: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            _ = tx.send(Event::DownloadStarted {
                kind: "artist".to_string(),
                id: artist_id.clone(),
                on,
            });
            let ids = crate::library::artist_song_ids(&dbs.library, &artist_id)
                .await
                .unwrap_or_default();
            if ids.is_empty() {
                _ = tx.send(Event::Error(
                    "This artist has no songs to download.".to_string(),
                ));
                return;
            }
            if on {
                for song in &ids {
                    if let Err(err) = crate::media_actions::download_one(&dbs, song).await {
                        _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                    }
                }
            } else {
                for song in &ids {
                    _ = crate::download::unmark(&dbs.download, song).await;
                }
                if let Err(err) = crate::media::remove_songs_media(&dbs.media, &ids).await {
                    _ = tx.send(Event::Error(format!(
                        "Could not remove the artist media: {err:#}"
                    )));
                }
            }
            send_download_settled(&tx);
        });
    }

    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::Relaxed);
    }

    /// UI request: retarget the output stream (or follow the default). The
    /// daemon owns the playback control, so this travels as an event.
    pub fn set_output_device(&self, device: Option<String>, follow_default: bool) {
        _ = self.tx.send(Event::OutputDevice {
            device,
            follow_default,
        });
    }

    /// UI request: enumerate the PipeWire sinks. Discovery blocks on a short
    /// private PipeWire connection, so it runs on the blocking pool.
    pub fn request_output_devices(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let result = tokio::task::spawn_blocking(crate::player::pipewire::list_output_devices)
                .await
                .unwrap_or_else(|error| Err(format!("output discovery task: {error}")));
            match result {
                Ok(devices) => {
                    _ = tx.send(Event::OutputDevices { devices });
                }
                Err(error) => {
                    _ = tx.send(Event::Error(format!("Could not list outputs: {error}")));
                }
            }
        });
    }

    /// UI request: change one stream-quality/cache field.
    pub fn set_stream_change(&self, change: crate::playback_settings::StreamChange) {
        _ = self.tx.send(Event::StreamChange(change));
    }

    /// UI request: arm/cancel the sleep timer.
    pub fn set_sleep_timer(&self, minutes: Option<u64>) {
        _ = self.tx.send(Event::SleepTimer(minutes));
    }

    /// UI request: enable/disable scrobbling.
    pub fn set_scrobble(&self, enabled: bool) {
        _ = self.tx.send(Event::Scrobble(enabled));
    }

    pub(crate) fn is_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    /// Applies a connectivity verdict from the background health probe.
    ///
    /// The bridge flag and the session flag move together: list loaders read
    /// the former, the cover/network paths read the latter. Only a transition
    /// reaches the UI, and it is deliberately toast-free — the 5 s probe must
    /// never spam the user.
    pub(crate) fn note_connection(&self, online: bool) {
        let was_online = !self.offline.swap(!online, Ordering::Relaxed);
        crate::session::set_online(online);
        if was_online != online {
            _ = self.tx.send(Event::Connection { online });
            if online {
                self.refresh_header();
            }
        }
    }
}
