//! Sync, wipe and container-maintenance requests.

use super::*;

impl Bridge {
    /// Run an incremental, deep, or clean-rebuild library sync.
    pub fn sync(&self, mode: SyncMode) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(client) = crate::session::client(&dbs).await else {
                let _ = tx.send(Event::Error("Sync needs a live session.".to_string()));
                return;
            };
            let client = Arc::new(client);
            if mode == SyncMode::Repair {
                let _ = tx.send(Event::SyncProgress {
                    message: "Repairing library…".to_string(),
                    done: 0,
                    total: 100,
                });
                if let Err(err) = crate::db::wipe_library(&dbs).await {
                    let _ = tx.send(Event::Error(format!("Library repair failed: {err:#}")));
                    return;
                }
            }
            let deep = mode != SyncMode::Incremental;
            let progress_tx = tx.clone();
            let report = match crate::index::sync_library(&dbs, client.clone(), deep, |progress| {
                let _ = progress_tx.send(Event::SyncProgress {
                    message: progress.message,
                    done: progress.done,
                    total: progress.total,
                });
            })
            .await
            {
                Ok(report) => report,
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Library sync failed: {err:#}")));
                    return;
                }
            };
            if mode != SyncMode::Incremental && report.covers_due {
                let _ = tx.send(Event::SyncProgress {
                    message: "Refreshing covers…".to_string(),
                    done: 90,
                    total: 100,
                });
                crate::index::cache_covers(&dbs, &client, &report.forced_covers).await;
            }
            let _ = tx.send(Event::LibrarySynced);
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
                    let _ = tx.send(Event::Wiped { scope });
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!("Wipe failed: {err:#}")));
                }
            }
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
                _ => (false, false, 0),
            };
            let _ = tx.send(Event::PageContainerState {
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
            let _ = tx.send(Event::DownloadStarted {
                kind: "artist".to_string(),
                id: artist_id.clone(),
                on,
            });
            let ids = crate::library::artist_song_ids(&dbs.library, &artist_id)
                .await
                .unwrap_or_default();
            if ids.is_empty() {
                let _ = tx.send(Event::Error(
                    "This artist has no songs to download.".to_string(),
                ));
                return;
            }
            if on {
                for song in &ids {
                    if let Err(err) = crate::media_actions::download_one(&dbs, song).await {
                        let _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                    }
                }
            } else {
                for song in &ids {
                    let _ = crate::download::unmark(&dbs.download, song).await;
                }
                if let Err(err) = crate::media::remove_songs_media(&dbs.media, &ids).await {
                    let _ = tx.send(Event::Error(format!(
                        "Could not remove the artist media: {err:#}"
                    )));
                }
            }
            let _ = tx.send(Event::DownloadSettled);
        });
    }

    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::Relaxed);
    }
}
