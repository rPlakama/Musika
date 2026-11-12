//! Library, search and windowing requests.

use super::*;

impl Bridge {
    pub fn refresh_header(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match crate::app::dbs() {
                Some(dbs) => match crate::accounts::active_account() {
                    Some(account) => {
                        let status = crate::library::status(&dbs.library).await.ok();
                        Event::Header {
                            server_url: account.server_url,
                            username: account.username,
                            server_type: account.server_type,
                            last_sync: status.as_ref().and_then(|status| status.last_sync.clone()),
                            counts: status
                                .map(|status| (status.artists, status.albums, status.songs))
                                .unwrap_or((0, 0, 0)),
                        }
                    }
                    None => {
                        let offline_available = crate::auth::downloaded_available().await;
                        return send_event(&tx, Event::LoginRequired { offline_available });
                    }
                },
                None => Event::Error("The library is not initialised".to_string()),
            };
            _ = tx.send(event);
        });
    }

    pub fn search(&self, query: String) {
        let generation = self.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation_counter = self.search_generation.clone();
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            // Debounce keystrokes: only the latest query runs the index search.
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            if generation_counter.load(Ordering::Relaxed) != generation {
                return;
            }
            let event =
                match search_rows(&query, offline, crate::model::SEARCH_FETCH_ROWS).await {
                    Ok((rows, most_played)) => {
                        crate::debug_log!("GPUI: search {:?} {} rows", query, rows.len());
                        Event::Search {
                            generation,
                            query,
                            rows,
                            most_played,
                        }
                    }
                    Err(err) => Event::Error(format!("Search failed: {err:#}")),
                };
            _ = tx.send(event);
        });
    }

    pub fn load_library(&self, scope: crate::model::Scope, order: usize) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        if scope == crate::model::Scope::Playlists {
            crate::app::runtime().spawn(async move {
                let event = match library_rows(scope, order).await {
                    Ok(rows) => {
                        crate::debug_log!("GPUI: {} {} rows", scope.label(), rows.len());
                        Event::Library {
                            generation,
                            scope,
                            order,
                            rows,
                        }
                    }
                    Err(err) => Event::Error(format!("Library load failed: {err:#}")),
                };
                _ = tx.send(event);
            });
            return generation;
        }
        crate::app::runtime().spawn(async move {
            let event = match crate::app::dbs() {
                Some(dbs) => {
                    let surface = scope.key();
                    match crate::rows::count(&dbs, surface, "", &order_name(scope, order), offline)
                        .await
                    {
                        Ok(count) => Event::LibraryCount {
                            generation,
                            scope,
                            order,
                            count,
                        },
                        Err(err) => Event::Error(format!("Library count failed: {err:#}")),
                    }
                }
                None => Event::Error("databases not ready".to_string()),
            };
            _ = tx.send(event);
        });
        generation
    }

    pub fn request_library_window(
        &self,
        generation: u64,
        scope: crate::model::Scope,
        order: usize,
        start: usize,
        count: usize,
    ) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let surface = scope.key();
            match crate::rows::window(
                &dbs,
                surface,
                "",
                &order_name(scope, order),
                start,
                count,
                offline,
            )
            .await
            {
                Ok(rows) => {
                    _ = tx.send(Event::LibraryWindow {
                        generation,
                        scope,
                        order,
                        start,
                        rows,
                    });
                }
                Err(err) => {
                    crate::debug_log!("GPUI: row window {surface} failed: {err:#}");
                }
            }
        });
    }

    pub fn preload_library(&self, scope: crate::model::Scope, order: usize) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(count) =
                crate::rows::count(&dbs, scope.key(), "", &order_name(scope, order), offline).await
            else {
                return;
            };
            if count == 0 {
                return;
            }
            let Ok(rows) = crate::rows::window(
                &dbs,
                scope.key(),
                "",
                &order_name(scope, order),
                0,
                50,
                offline,
            )
            .await
            else {
                return;
            };
            _ = tx.send(Event::Preload {
                scope,
                order,
                count,
                rows,
            });
        });
    }
}

impl Bridge {
    /// Async scoped suggestions for `:`/IPC commands. Uses the same unified
    /// search result as the Search overlay; stale generations are ignored by
    /// the caller.
    pub fn request_command_suggestions(
        &self,
        generation: u64,
        scope: crate::ipc_command::SuggestScope,
        query: String,
        start: usize,
        end: usize,
    ) {
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        crate::app::runtime().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
            let Some(dbs) = crate::app::dbs() else { return };
            let Ok(results) = crate::browse::search(&dbs, &query, offline, 8).await else {
                _ = tx.send(Event::CommandSuggestions {
                    generation,
                    start,
                    end,
                    rows: Vec::new(),
                });
                return;
            };
            let mut rows: Vec<(String, String)> = Vec::new();
            let mut push = |title: &str, detail: &str, kind: &str| {
                if rows.len() >= 8 || title.is_empty() {
                    return;
                }
                rows.push((
                    title.to_string(),
                    if detail.is_empty() {
                        kind.to_string()
                    } else {
                        format!("{kind} · {detail}")
                    },
                ));
            };
            match scope {
                crate::ipc_command::SuggestScope::Artist => {
                    for artist in &results.artists {
                        push(&artist.name, "", "Artist");
                    }
                }
                crate::ipc_command::SuggestScope::Album => {
                    for album in &results.albums {
                        push(&album.title, &album.artist, "Album");
                    }
                }
                crate::ipc_command::SuggestScope::Track => {
                    for song in &results.songs {
                        let subtitle = song.subtitle();
                        push(&song.title, song.artist.as_deref().unwrap_or(&subtitle), "Track");
                    }
                }
                crate::ipc_command::SuggestScope::Playlist => {
                    for playlist in &results.playlists {
                        push(&playlist.name, &playlist.subtitle, "Playlist");
                    }
                }
                crate::ipc_command::SuggestScope::Any => {
                    for song in &results.songs {
                        let subtitle = song.subtitle();
                        push(&song.title, song.artist.as_deref().unwrap_or(&subtitle), "Track");
                    }
                    for album in &results.albums {
                        push(&album.title, &album.artist, "Album");
                    }
                    for artist in &results.artists {
                        push(&artist.name, "", "Artist");
                    }
                    for playlist in &results.playlists {
                        push(&playlist.name, &playlist.subtitle, "Playlist");
                    }
                }
            }
            _ = tx.send(Event::CommandSuggestions {
                generation,
                start,
                end,
                rows,
            });
        });
    }
}
