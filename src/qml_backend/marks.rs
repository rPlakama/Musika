// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Download mark-window reads, the per-id mark truth and its debounced reader.
//!
//! The QML slots stay in `qml.rs` (qtbridge requires one annotated impl);
//! these are their bodies and the shared window machinery.

use std::sync::atomic::Ordering;

use qtbridge::QmlObject;

use super::download::apply_window;
use super::{Backend, MARK_WINDOW_GEN, MarkState, add_mark_id, remove_mark_id};

impl Backend {
    pub(super) fn store_and_refresh(&mut self, songs: Vec<String>, albums: Vec<String>) {
        self.download_window_songs = songs.clone();
        self.download_window_albums = albums.clone();
        const RECENT_REQUESTS: usize = 12;
        // An identical re-sent window must not flood the union.
        if self
            .recent_mark_requests
            .back()
            .is_none_or(|(s, a)| *s != songs || *a != albums)
        {
            self.recent_mark_requests
                .push_back((songs.clone(), albums.clone()));
            while self.recent_mark_requests.len() > RECENT_REQUESTS {
                self.recent_mark_requests.pop_front();
            }
        }
        self.prune_mark_states();
        self.refresh_window(songs, albums);
    }

    pub(super) fn set_mark_state(&mut self, id: &str, state: Option<MarkState>) {
        match state {
            Some(state) => {
                self.mark_states.insert(id.to_string(), state);
            }
            None => {
                self.mark_states.remove(id);
            }
        }
    }

    /// One id's state patch: keeps the exposed sets incremental instead of
    /// rebuilding (and re-sorting) the whole mark truth on every fetch step.
    pub(super) fn patch_mark_state(&mut self, id: &str, state: Option<MarkState>) {
        let old = self.mark_states.get(id).copied();
        if old == state {
            return;
        }
        self.set_mark_state(id, state);
        let mut done_changed = false;
        let mut cached_changed = false;
        let mut pending_changed = false;
        if let Some(old) = old {
            match old {
                MarkState::SongDone => done_changed |= remove_mark_id(&mut self.downloaded_ids, id),
                MarkState::AlbumCached => {
                    cached_changed |= remove_mark_id(&mut self.cached_album_ids, id)
                }
                MarkState::SongInFlight => {
                    pending_changed |= remove_mark_id(&mut self.downloading_ids, id)
                }
            }
        }
        if let Some(state) = state {
            match state {
                MarkState::SongDone => done_changed |= add_mark_id(&mut self.downloaded_ids, id),
                MarkState::AlbumCached => {
                    cached_changed |= add_mark_id(&mut self.cached_album_ids, id)
                }
                MarkState::SongInFlight => {
                    pending_changed |= add_mark_id(&mut self.downloading_ids, id)
                }
            }
        }
        if done_changed {
            self.downloaded_ids_changed();
        }
        if cached_changed {
            self.cached_album_ids_changed();
        }
        if pending_changed {
            self.downloading_ids_changed();
        }
    }

    /// Drops mark entries that fell out of every recent surface window. The
    /// map therefore stays bounded by the recently requested rows instead of
    /// accumulating every id ever scrolled past.
    pub(super) fn prune_mark_states(&mut self) {
        let mut allowed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (songs, albums) in &self.recent_mark_requests {
            allowed.extend(songs.iter().cloned());
            allowed.extend(albums.iter().cloned());
        }
        let before = self.mark_states.len();
        self.mark_states.retain(|id, _| allowed.contains(id));
        if self.mark_states.len() != before {
            self.rebuild_mark_sets();
        }
    }

    /// Re-derives the three exposed sets from the state map (sorted,
    /// notify-on-change).
    pub(super) fn rebuild_mark_sets(&mut self) {
        let mut done: Vec<String> = Vec::new();
        let mut cached: Vec<String> = Vec::new();
        let mut pending: Vec<String> = Vec::new();
        for (id, state) in &self.mark_states {
            match state {
                MarkState::SongDone => done.push(id.clone()),
                MarkState::AlbumCached => cached.push(id.clone()),
                MarkState::SongInFlight => pending.push(id.clone()),
            }
        }
        done.sort();
        cached.sort();
        pending.sort();
        if done != self.downloaded_ids {
            self.downloaded_ids = done;
            self.downloaded_ids_changed();
        }
        if cached != self.cached_album_ids {
            self.cached_album_ids = cached;
            self.cached_album_ids_changed();
        }
        if pending != self.downloading_ids {
            self.downloading_ids = pending;
            self.downloading_ids_changed();
        }
    }

    pub(super) fn hold_download_request(&mut self) {
        const DOWNLOAD_HOLD: std::time::Duration = std::time::Duration::from_millis(800);
        self.download_request_until = Some(std::time::Instant::now() + DOWNLOAD_HOLD);
        if !self.while_download_request {
            self.while_download_request = true;
            self.while_download_request_changed();
        }
    }

    /// Ends one song's request: immediately when the hold has passed, else
    /// after the remaining hold (through the runtime + invoker, so the write
    /// lands on the main thread).
    pub(super) fn settle_download_request(&mut self, song_id: String) {
        let delay = self
            .download_request_until
            .map(|until| until.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or_default();
        if delay.is_zero() {
            self.apply_download_settle(song_id);
            return;
        }
        let invoker = self.get_qml_method_invoker();
        crate::app::runtime().spawn(async move {
            tokio::time::sleep(delay).await;
            qtbridge::invoke_method!(invoker, "settleDownloadRequest", song_id);
        });
    }

    pub(super) fn apply_download_settle(&mut self, song_id: String) {
        crate::offline::clear_cache();
        let touched = self
            .mark_states
            .get(&song_id)
            .is_some_and(|state| *state == MarkState::SongInFlight);
        if touched {
            // The completion already wrote SongDone for finished fetches;
            // only a still-in-flight mark falls away here.
            self.patch_mark_state(&song_id, None);
        }
        if self.downloading_ids.is_empty() && self.while_download_request {
            self.while_download_request = false;
            self.while_download_request_changed();
        }
    }

    pub(super) fn refresh_download_window(&mut self) {
        // Re-read over the union of the recent requests: a download's
        // success must re-evaluate every surface that is showing rows —
        // album ids included, even when the last request was a songs one.
        if self.recent_mark_requests.is_empty() {
            return;
        }
        let songs: Vec<String> = self
            .recent_mark_requests
            .iter()
            .flat_map(|(songs, _)| songs.iter().cloned())
            .collect();
        let albums: Vec<String> = self
            .recent_mark_requests
            .iter()
            .flat_map(|(_, albums)| albums.iter().cloned())
            .collect();
        self.refresh_window(songs, albums);
    }

    /// One refresh read over explicit windows.
    pub(super) fn refresh_window(&mut self, songs: Vec<String>, albums: Vec<String>) {
        // PATH-5: accumulate every request in the debounce window and read
        // their union once; the newest generation owns the apply.
        let (songs_snapshot, albums_snapshot) = {
            let mut pending = self
                .mark_pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            pending.songs.extend(songs);
            pending.albums.extend(albums);
            (
                pending.songs.iter().cloned().collect::<Vec<_>>(),
                pending.albums.iter().cloned().collect::<Vec<_>>(),
            )
        };
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        let pending = std::sync::Arc::clone(&self.mark_pending);
        let generation = MARK_WINDOW_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::app::runtime().spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            if MARK_WINDOW_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            apply_window(&dbs, &songs_snapshot, &albums_snapshot, generation, invoker).await;
            // A newer request owns the rest; only a still-current apply
            // retires its snapshot from the pending union.
            if MARK_WINDOW_GEN.load(Ordering::Relaxed) == generation {
                let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
                for id in &songs_snapshot {
                    pending.songs.remove(id);
                }
                for id in &albums_snapshot {
                    pending.albums.remove(id);
                }
            }
        });
    }
}
