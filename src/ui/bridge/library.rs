//! Library, search and windowing requests.

use super::*;

impl Bridge {
    pub fn refresh_header(&self) {
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let event = match crate::app::dbs() {
                Some(dbs) => match crate::sqlite::load_credentials(&dbs.credentials).await {
                    Ok(Some(credentials)) => {
                        let status = crate::library::status(&dbs.library).await.ok();
                        Event::Header {
                            server_url: credentials.server_url,
                            username: credentials.username,
                            server_type: credentials.server_type,
                            last_sync: status.as_ref().and_then(|status| status.last_sync.clone()),
                            counts: status
                                .map(|status| (status.artists, status.albums, status.songs))
                                .unwrap_or((0, 0, 0)),
                        }
                    }
                    Ok(None) => {
                        let offline_available = crate::auth::downloaded_available().await;
                        return send_event(&tx, Event::LoginRequired { offline_available });
                    }
                    Err(err) => Event::Error(format!("Could not read credentials: {err:#}")),
                },
                None => Event::Error("The library is not initialised".to_string()),
            };
            let _ = tx.send(event);
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
                match search_rows(&query, offline, super::super::state::SEARCH_FETCH_ROWS).await {
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
            let _ = tx.send(event);
        });
    }

    pub fn load_library(&self, scope: super::super::state::Scope, order: usize) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        let offline = self.offline.load(Ordering::Relaxed);
        if scope == super::super::state::Scope::Playlists {
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
                let _ = tx.send(event);
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
            let _ = tx.send(event);
        });
        generation
    }

    pub fn request_library_window(
        &self,
        generation: u64,
        scope: super::super::state::Scope,
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
                Ok(value) => {
                    let rows = value["rows"]
                        .as_array()
                        .map(|rows| {
                            rows.iter()
                                .map(|row| row_from_json(scope, row))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(Event::LibraryWindow {
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

    pub fn preload_library(&self, scope: super::super::state::Scope, order: usize) {
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
            let Ok(value) = crate::rows::window(
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
            let rows = value["rows"]
                .as_array()
                .map(|rows| rows.iter().map(|row| row_from_json(scope, row)).collect())
                .unwrap_or_default();
            let _ = tx.send(Event::Preload {
                scope,
                order,
                count,
                rows,
            });
        });
    }
}
