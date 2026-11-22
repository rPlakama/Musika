//! Quick action requests (favourite/rating/download/queue/shuffle).

use super::*;

use crate::model::RowKind;

impl Bridge {
    pub fn quick_favorite(&self, kind: String, id: String, on: bool) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Some(row_kind) =
                RowKind::from_api_kind(&kind).filter(|kind| *kind != RowKind::Playlist)
            else {
                _ = tx.send(Event::Error(format!(
                    "Could not update the favourite: unknown favourite kind: {kind}"
                )));
                return;
            };
            let result = async {
                let client = crate::session::client(&dbs).await?;
                if on {
                    match row_kind {
                        RowKind::Song => client.star(&[id.as_str()], &[], &[]).await,
                        RowKind::Album => client.star(&[], &[id.as_str()], &[]).await,
                        RowKind::Artist | RowKind::Playlist => {
                            client.star(&[], &[], &[id.as_str()]).await
                        }
                    }
                } else {
                    match row_kind {
                        RowKind::Song => client.unstar(&[id.as_str()], &[], &[]).await,
                        RowKind::Album => client.unstar(&[], &[id.as_str()], &[]).await,
                        RowKind::Artist | RowKind::Playlist => {
                            client.unstar(&[], &[], &[id.as_str()]).await
                        }
                    }
                }
            };
            if let Err(err) = result.await {
                _ = tx.send(Event::Error(format!(
                    "Could not update the favourite: {err:#}"
                )));
                return;
            }
            let local = match row_kind {
                RowKind::Song => crate::library::set_song_starred(&dbs.library, &id, on).await,
                RowKind::Album => crate::library::set_album_starred(&dbs.library, &id, on).await,
                RowKind::Artist | RowKind::Playlist => {
                    crate::artist_user::set_starred(&dbs.artist_user, &id, on).await
                }
            };
            if let Err(err) = local {
                _ = tx.send(Event::Error(format!(
                    "Could not save the favourite: {err:#}"
                )));
                return;
            }
            _ = tx.send(Event::RowsPatched {
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
            let Some(row_kind) =
                RowKind::from_api_kind(&kind).filter(|kind| *kind != RowKind::Playlist)
            else {
                _ = tx.send(Event::Error(format!(
                    "Could not rate: unknown rating kind: {kind}"
                )));
                return;
            };
            let result = async {
                let client = crate::session::client(&dbs).await?;
                client.set_rating(&id, level).await?;
                match row_kind {
                    RowKind::Artist => {
                        crate::artist_user::set_rating(&dbs.artist_user, &id, level).await
                    }
                    RowKind::Album => {
                        crate::library::set_album_rating(&dbs.library, &id, level).await
                    }
                    RowKind::Song | RowKind::Playlist => Ok(()),
                }
            }
            .await;
            if let Err(err) = result {
                _ = tx.send(Event::Error(format!("Could not rate: {err:#}")));
                return;
            }
            _ = tx.send(Event::RowsPatched {
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
            _ = tx.send(Event::DownloadStarted {
                kind: kind.clone(),
                id: id.clone(),
                on,
            });
            let _settled = DownloadSettledGuard::new(tx.clone());
            let row_kind = RowKind::from_api_kind(&kind);
            if !on {
                let result = if row_kind == Some(RowKind::Album) {
                    crate::media_actions::album_container_off(&dbs, &id).await;
                    let songs = crate::library::album_song_ids(&dbs.library, &id)
                        .await
                        .unwrap_or_default();
                    let mut first_err = None;
                    for song in &songs {
                        if let Err(err) = crate::download::unmark(&dbs.download, song).await
                            && first_err.is_none()
                        {
                            first_err = Some(err);
                        }
                    }
                    if !songs.is_empty()
                        && let Err(err) = crate::media::remove_songs_media(&dbs.media, &songs).await
                        && first_err.is_none()
                    {
                        first_err = Some(err);
                    }
                    match first_err {
                        Some(err) => Err(err),
                        None => Ok(()),
                    }
                } else if row_kind == Some(RowKind::Song) {
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
                    _ = tx.send(Event::Error(format!("Could not remove download: {err:#}")));
                }
                return;
            }
            let songs = if row_kind == Some(RowKind::Song) {
                vec![id.clone()]
            } else if row_kind == Some(RowKind::Album) {
                crate::library::album_song_ids_of(&dbs.library, std::slice::from_ref(&id))
                    .await
                    .ok()
                    .and_then(|mut map| map.remove(&id))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if songs.is_empty() {
                _ = tx.send(Event::Error(
                    "No songs to download for this item".to_string(),
                ));
                return;
            }
            if row_kind == Some(RowKind::Song) {
                if let Err(err) = crate::media_actions::download_one(&dbs, &id).await {
                    _ = tx.send(Event::Error(format!("Download failed: {err:#}")));
                }
                return;
            }
            let rule_id = if row_kind == Some(RowKind::Album) {
                crate::media::set_rule(&dbs.media, "album", &id).await.ok()
            } else {
                None
            };
            let mut failed = 0usize;
            let mut first_err: Option<String> = None;
            for song in &songs {
                if let Some(rule_id) = rule_id.as_deref() {
                    _ = crate::download::mark_queued(&dbs.download, song, Some(rule_id)).await;
                }
                if let Err(err) = crate::media_actions::download_one(&dbs, song).await {
                    failed += 1;
                    if first_err.is_none() {
                        first_err = Some(format!("{err:#}"));
                    }
                }
            }
            if let Some(first_err) = first_err {
                _ = tx.send(Event::Error(format!(
                    "{failed} track download(s) failed: {first_err}"
                )));
            }
        });
    }

    pub fn quick_queue_add(&self, kind: String, id: String) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let queued = match RowKind::from_api_kind(&kind) {
                Some(RowKind::Playlist) => crate::queue::playlist_tracks(&dbs, &id, offline, 0)
                    .await
                    .map(|(tracks, _)| tracks),
                Some(row_kind) => {
                    crate::queue::quick_tracks(&dbs, row_kind.api_kind(), &id, offline).await
                }
                // Unknown kinds keep the pre-parse behaviour: the queue
                // helper rejects them with its own message.
                None => crate::queue::quick_tracks(&dbs, &kind, &id, offline).await,
            };
            emit_append(&tx, queued, "Could not add to queue");
        });
    }

    /// VI-8a: resolve the row clipboard and insert it at `at` in the live
    /// queue (paste below/above the queue cursor).
    pub fn paste_queue(&self, kind: String, ids: Vec<String>, at: usize) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::queue::quick_tracks_many(&dbs, &kind, &ids, offline).await {
                Ok(tracks) if tracks.is_empty() => {
                    _ = tx.send(Event::Error(
                        "Could not paste: no tracks matched the row clipboard".to_string(),
                    ));
                }
                Ok(tracks) => {
                    _ = tx.send(Event::QueuePaste { tracks, at });
                }
                Err(err) => {
                    _ = tx.send(Event::Error(format!("Could not paste: {err:#}")));
                }
            }
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
            _ = tx.send(Event::OpenPage {
                descriptor: crate::model::PageDescriptor::new(
                    crate::model::PageKind::Artist,
                    artist_id,
                ),
            });
        });
    }

    pub fn quick_play_playlist(&self, id: String, start: usize) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue_pair(
                &tx,
                crate::queue::playlist_tracks(&dbs, &id, offline, start as i64)
                    .await
                    .map(|(tracks, start)| (tracks, start as usize)),
                "Could not play the playlist",
            );
        });
    }

    /// Replace the queue with one shuffled artist pass.
    pub fn shuffle_album(&self, album_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue(
                &tx,
                crate::queue::album_shuffle_tracks(&dbs, &album_id, offline).await,
                0,
                "Could not shuffle the album",
            );
        });
    }

    pub fn shuffle_artist(&self, artist_id: String) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            emit_queue(
                &tx,
                crate::queue::artist_shuffle_tracks(&dbs, &artist_id, offline).await,
                0,
                "Could not shuffle the artist",
            );
        });
    }
}

impl Bridge {
    /// Named shuffle target: resolve via search, then reuse the id path.
    pub fn shuffle_album_query(&self, query: String) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = async {
                let id = crate::download_command::resolve_named_id(
                    &dbs,
                    crate::ipc_command::DownloadScope::Album,
                    &query,
                )
                .await?;
                crate::queue::album_shuffle_tracks(&dbs, &id, offline).await
            }
            .await;
            emit_queue(&tx, result, 0, "Could not shuffle the album");
        });
    }

    /// Named shuffle target: resolve via search, then reuse the id path.
    pub fn shuffle_artist_query(&self, query: String) {
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let result = async {
                let id = crate::download_command::resolve_named_id(
                    &dbs,
                    crate::ipc_command::DownloadScope::Artist,
                    &query,
                )
                .await?;
                crate::queue::artist_shuffle_tracks(&dbs, &id, offline).await
            }
            .await;
            emit_queue(&tx, result, 0, "Could not shuffle the artist");
        });
    }
}
