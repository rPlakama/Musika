//! Quick action requests (favourite/rating/download/queue/shuffle).

use super::*;

impl Bridge {
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
                let _ = tx.send(Event::Error(format!(
                    "Could not update the favourite: {err:#}"
                )));
                return;
            }
            let local = match kind.as_str() {
                "song" => crate::library::set_song_starred(&dbs.library, &id, on).await,
                "album" => crate::library::set_album_starred(&dbs.library, &id, on).await,
                _ => crate::artist_user::set_starred(&dbs.artist_user, &id, on).await,
            };
            if let Err(err) = local {
                let _ = tx.send(Event::Error(format!(
                    "Could not save the favourite: {err:#}"
                )));
                return;
            }
            let row_kind = match kind.as_str() {
                "song" => super::super::state::RowKind::Song,
                "album" => super::super::state::RowKind::Album,
                _ => super::super::state::RowKind::Artist,
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
                super::super::state::RowKind::Artist
            } else {
                super::super::state::RowKind::Album
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
                    let media =
                        crate::media::remove_songs_media(&dbs.media, std::slice::from_ref(&id))
                            .await;
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
                let _ = tx.send(Event::Error(
                    "No songs to download for this item".to_string(),
                ));
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
            emit_append(&tx, queued, "Could not add to queue");
        });
    }

    pub fn quick_go_artist(&self, album_id: String) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(Some(artist_id)) =
                crate::library::album_artist_id(&dbs.library, &album_id).await
            else {
                return;
            };
            let _ = tx.send(Event::OpenPage {
                descriptor: super::super::state::PageDescriptor::new(
                    super::super::state::PageKind::Artist,
                    artist_id,
                ),
            });
        });
    }

    pub fn quick_play_playlist(&self, id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue_pair(
                &tx,
                crate::queue::playlist_tracks_json(&dbs, &id, offline, 0)
                    .await
                    .map(|(json, start)| (json, start as usize)),
                "Could not play the playlist",
            );
        });
    }

    /// Replace the queue with one shuffled artist pass.
    pub fn shuffle_album(&self, album_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue(
                &tx,
                crate::queue::album_shuffle_json(&dbs, &album_id, offline).await,
                0,
                "Could not shuffle the album",
            );
        });
    }

    pub fn shuffle_artist(&self, artist_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue(
                &tx,
                crate::queue::artist_shuffle_json(&dbs, &artist_id, offline).await,
                0,
                "Could not shuffle the artist",
            );
        });
    }
}
