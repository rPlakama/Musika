//! Page payload and notes requests.

use super::*;

impl Bridge {
    pub fn load_page(&self, descriptor: crate::model::PageDescriptor) {
        let generation = self.page_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        let offline = self.offline();
        crate::app::runtime().spawn(async move {
            let event = match page_data(&descriptor, offline).await {
                Ok((header, rows)) => {
                    crate::debug_log!(
                        "bridge: {} page {} rows",
                        descriptor.kind.label(),
                        rows.len()
                    );
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
                    id: album_id.clone(),
                    kind: NotesKind::Album,
                    notes: plain_notes(notes),
                });
                // A negative marker (empty notes + timestamp) is a real cache
                // hit: the provider answered "no description", and the TTL
                // must keep the next page open off the network (D-27).
                if info_cache_is_fresh(*fetched_at, crate::consts::now_secs()) {
                    return;
                }
            }
            if !crate::session::online() {
                if cached.is_none() {
                    _ = tx.send(Event::Notes {
                        id: album_id,
                        kind: NotesKind::Album,
                        notes: String::new(),
                    });
                }
                return;
            }
            // `None` = the request failed; `Some("")` = the provider answered
            // with no description, which is cached as a negative marker below.
            let fetched = match crate::session::client(&dbs).await {
                Ok(client) => client
                    .get_album_info2(&album_id)
                    .await
                    .ok()
                    .map(|info| info.notes.unwrap_or_default()),
                Err(_) => None,
            };
            if let Some(fresh) = &fetched
                && let Err(err) =
                    crate::library::upsert_album_info(&dbs.library, &album_id, fresh).await
            {
                crate::debug_log!("bridge: album notes cache failed: {err:#}");
            }
            let notes = fetched
                .or_else(|| cached.map(|(notes, _)| notes))
                .unwrap_or_default();
            _ = tx.send(Event::Notes {
                id: album_id,
                kind: NotesKind::Album,
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
                    id: artist_id.clone(),
                    kind: NotesKind::Artist,
                    notes: plain_notes(&row.biography),
                });
                if row
                    .fetched_at
                    .parse::<i64>()
                    .is_ok_and(|at| info_cache_is_fresh(at, crate::consts::now_secs()))
                {
                    return;
                }
            }
            if !crate::session::online() {
                if cached.is_none() {
                    _ = tx.send(Event::Notes {
                        id: artist_id,
                        kind: NotesKind::Artist,
                        notes: String::new(),
                    });
                }
                return;
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
            // `Some` similar list = the provider answered (possibly with no
            // artists): cache the biography even when the provider sent none,
            // so the empty row acts as a negative cache marker (D-27). A
            // failed request (`None`) must never mark the biography as fresh.
            if let Some(similar) = similar {
                if let Err(err) =
                    crate::library::upsert_artist_similar(&dbs.library, &artist_id, &similar).await
                {
                    crate::debug_log!("bridge: artist similar graph cache failed: {err:#}");
                }
                if let Err(err) =
                    crate::library::upsert_artist_info(&dbs.library, &artist_id, &fresh, "").await
                {
                    crate::debug_log!("bridge: artist notes cache failed: {err:#}");
                }
            }
            let notes = if fresh.is_empty() {
                cached.map(|row| row.biography).unwrap_or_default()
            } else {
                fresh
            };
            _ = tx.send(Event::Notes {
                id: artist_id,
                kind: NotesKind::Artist,
                notes: plain_notes(&notes),
            });
        });
    }
}

/// Whether a cached info row is still inside [`crate::consts::INFO_TTL_SECS`].
///
/// A negative-cache marker (the provider answered, with no notes) is as fresh
/// as a row holding text: the TTL is about when the question was last asked,
/// not about what the answer was (D-27).
fn info_cache_is_fresh(fetched_at: i64, now: i64) -> bool {
    now.saturating_sub(fetched_at) < crate::consts::INFO_TTL_SECS
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    /// A migrated in-memory `library.db` for the cache round trips.
    async fn library_pool() -> sqlx::SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("memory database url");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("memory pool");
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .expect("library migrations");
        pool
    }

    #[test]
    fn info_ttl_boundary_marks_a_fresh_negative_cache_as_fresh() {
        let now = 1_700_000_000;
        // Written just now (empty or not): the network must be skipped.
        assert!(info_cache_is_fresh(now, now));
        assert!(info_cache_is_fresh(
            now - crate::consts::INFO_TTL_SECS + 1,
            now
        ));
        // Exactly at and past the TTL: refetch.
        assert!(!info_cache_is_fresh(
            now - crate::consts::INFO_TTL_SECS,
            now
        ));
        assert!(!info_cache_is_fresh(
            now - 2 * crate::consts::INFO_TTL_SECS,
            now
        ));
    }

    #[tokio::test]
    async fn empty_provider_notes_persist_as_a_fresh_negative_marker() {
        let pool = library_pool().await;

        // Album: an empty description fetched from the provider is stored
        // with `fetched_at` so the next page open skips the network.
        crate::library::upsert_album_info(&pool, "al1", "")
            .await
            .expect("store album marker");
        let (notes, fetched_at) = crate::library::album_info(&pool, "al1")
            .await
            .expect("load album marker")
            .expect("album marker row");
        assert!(notes.is_empty());
        assert!(info_cache_is_fresh(fetched_at, crate::consts::now_secs()));

        // Artist: same contract through the separate `artist_info` table.
        crate::library::upsert_artist_info(&pool, "ar1", "", "")
            .await
            .expect("store artist marker");
        let row = crate::library::artist_info(&pool, "ar1")
            .await
            .expect("load artist marker")
            .expect("artist marker row");
        assert!(row.biography.is_empty());
        let fetched_at = row.fetched_at.parse::<i64>().expect("artist timestamp");
        assert!(info_cache_is_fresh(fetched_at, crate::consts::now_secs()));
    }

    /// The new discriminator is serde-defaulted: an event serialized before
    /// D-27 (no `kind`) still resolves to the only kind that existed then.
    #[test]
    fn legacy_notes_payload_without_kind_reads_as_album_notes() {
        let event: Event = serde_json::from_str(r#"{"Notes":{"id":"al1","notes":"old"}}"#)
            .expect("legacy notes payload");
        let Event::Notes { id, kind, notes } = event else {
            panic!("expected a Notes event");
        };
        assert_eq!(id, "al1");
        assert_eq!(kind, NotesKind::Album);
        assert_eq!(notes, "old");
    }
}
