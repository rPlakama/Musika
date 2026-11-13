//! Page payload and notes requests.

use super::*;

impl Bridge {
    pub fn load_page(&self, descriptor: crate::model::PageDescriptor) {
        let generation = self.page_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let event = match page_data(&descriptor, offline).await {
                Ok((header, rows)) => {
                    crate::debug_log!("GPUI: {} page {} rows", descriptor.kind.label(), rows.len());
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
                _ = tx.send(Event::Notes {
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
            _ = tx.send(Event::Notes {
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
                _ = tx.send(Event::Notes {
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
            // One request feeds both caches: the biography shown on the page
            // and the similar-artist graph the local radio engine prefers.
            // `getArtistInfo2` is the ID3 variant; `getArtistInfo` is the
            // legacy fallback. `includeNotPresent=false` keeps the graph to
            // artists this server can actually return songs for.
            let (fresh, similar) = match crate::session::client(&dbs).await {
                Ok(client) => match client
                    .get_artist_info2(&artist_id, Some(15), Some(false))
                    .await
                {
                    Ok(info) => (
                        info.info.biography.unwrap_or_default(),
                        Some(
                            info.similar_artist
                                .into_iter()
                                .map(|artist| crate::library::SimilarArtist {
                                    id: artist.id,
                                    name: artist.name,
                                })
                                .collect::<Vec<_>>(),
                        ),
                    ),
                    Err(_) => match client.get_artist_info(&artist_id, Some(15), None).await {
                        Ok(info) => (
                            info.info.biography.unwrap_or_default(),
                            Some(
                                info.similar_artist
                                    .into_iter()
                                    .map(|artist| crate::library::SimilarArtist {
                                        id: artist.id,
                                        name: artist.name,
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        ),
                        Err(_) => (String::new(), None),
                    },
                },
                Err(_) => (String::new(), None),
            };
            if let Some(similar) = similar
                && let Err(err) = crate::library::upsert_artist_similar(
                    &dbs.library,
                    &artist_id,
                    &similar,
                )
                .await
            {
                crate::debug_log!("GPUI: artist similar graph cache failed: {err:#}");
            }
            if !fresh.is_empty() {
                _ =
                    crate::library::upsert_artist_info(&dbs.library, &artist_id, &fresh, "").await;
            }
            let notes = if fresh.is_empty() {
                cached.map(|row| row.biography).unwrap_or_default()
            } else {
                fresh
            };
            _ = tx.send(Event::Notes {
                album_id: artist_id,
                notes: plain_notes(&notes),
            });
        });
    }
}
