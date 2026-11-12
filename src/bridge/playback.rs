//! Restore, lyrics and playback requests.

use super::*;

impl Bridge {
    pub fn restore(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(status)) = crate::status::load(&dbs.status).await else {
                return;
            };
            let resume = crate::queue::resume(&dbs, &status, false).await.ok();
            let tracks = resume
                .as_ref()
                .map(|resume| resume.tracks.clone())
                .unwrap_or_default();
            let start = resume.as_ref().map(|resume| resume.start).unwrap_or(0);
            let position = resume
                .as_ref()
                .map(|resume| resume.position)
                .unwrap_or(status.position);
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
            emit_queue(
                &tx,
                crate::queue::album_tracks(&dbs, &album_id, offline).await,
                start,
                "Could not play album",
            );
        });
    }

    /// IPC `play_song_in_album`: load the song's album queue at that song.
    /// Tray `random`: pick a random non-empty album from `library.db` and
    /// start it in album order, opening the album page.
    pub fn play_random_album(&self) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let id = match crate::library::random_album_id(&dbs.library).await {
                Ok(Some(id)) => id,
                Ok(None) => {
                    let _ = tx.send(Event::Error("The library has no albums to pick.".to_string()));
                    return;
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!(
                        "Could not pick a random album: {err:#}"
                    )));
                    return;
                }
            };
            match crate::queue::album_tracks(&dbs, &id, offline).await {
                Ok(tracks) if !tracks.is_empty() => {
                    let _ = tx.send(Event::Queue { tracks, start: 0 });
                    let _ = tx.send(Event::OpenPage {
                        descriptor: crate::model::PageDescriptor::new(
                            crate::model::PageKind::Album,
                            id,
                        ),
                    });
                }
                Ok(_) => {
                    let _ = tx.send(Event::Error(
                        "The random album has no playable tracks.".to_string(),
                    ));
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!(
                        "Could not load the random album: {err:#}"
                    )));
                }
            }
        });
    }

    pub fn play_song_in_album(&self, song_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Some(album_id) =
                crate::library::song_album_ids(&dbs.library, std::slice::from_ref(&song_id))
                    .await
                    .ok()
                    .and_then(|map| map.get(&song_id).cloned())
            else {
                let _ = tx.send(Event::Error("That song has no album to open.".to_string()));
                return;
            };
            match crate::queue::album_tracks(&dbs, &album_id, offline).await {
                Ok(tracks) => {
                    let start = tracks
                        .iter()
                        .position(|track| track.id == song_id)
                        .unwrap_or(0);
                    let _ = tx.send(Event::Queue { tracks, start });
                    let _ = tx.send(Event::OpenPage {
                        descriptor: crate::model::PageDescriptor::new(
                            crate::model::PageKind::Album,
                            album_id,
                        ),
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::Error(format!(
                        "Could not play the song's album: {err:#}"
                    )));
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
}
