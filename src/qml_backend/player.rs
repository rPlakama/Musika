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

//! Queue publishing and the player→bridge sync used by the QML slots.

use std::sync::atomic::Ordering;

use qtbridge::QmlObject;

use super::{Backend, QUEUE_EMIT_GEN, QUEUE_ROWS_HASH, QueueCoversEnvelope, QueueRows, emit};

impl Backend {
    /// The player queue as a JSON id array (P-6 S3 snapshot).
    pub(super) fn queue_ids_json(&self) -> String {
        let ids: Vec<&str> = self
            .player
            .queue()
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        serde_json::to_string(&ids).unwrap_or_default()
    }

    pub(super) fn refresh_queue(&mut self) {
        let invoker = self.get_qml_method_invoker();
        // COW snapshot: the task borrows the rows instead of cloning three
        // strings per queued track on the Qt thread.
        let queue = self.player.queue_snapshot();
        crate::row_cache::set_queue_snapshot(queue.clone());
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        crate::app::runtime().spawn(async move {
            let album_ids: Vec<&str> = queue
                .iter()
                .map(|track| track.album_id.as_str())
                .filter(|id| !id.is_empty())
                .collect();
            let albums = crate::library::album_covers_refs(&dbs.library, &album_ids)
                .await
                .unwrap_or_default();
            let song_ids: Vec<&str> = queue.iter().map(|track| track.id.as_str()).collect();
            let songs = crate::library::songs_by_ids_refs(&dbs.library, &song_ids)
                .await
                .unwrap_or_default();
            let cover_ids: Vec<String> = queue
                .iter()
                .map(|track| {
                    albums
                        .get(track.album_id.as_str())
                        .cloned()
                        .unwrap_or_else(|| track.cover_art.clone())
                })
                .filter(|id| !id.is_empty())
                .collect();
            let covers = crate::covers::url_map_of(&dbs, &cover_ids)
                .await
                .unwrap_or_default();
            let json = serde_json::to_string(&QueueCoversEnvelope {
                covers: &covers,
                albums: &albums,
                songs: &songs,
            })
            .unwrap_or_default();
            qtbridge::invoke_method!(invoker, "applyQueueCovers", json);
        });
        self.request_colors();
    }

    /// Publishes the queue snapshot with resolved covers and stars. A clean
    /// revision coalesces unless `force` is set; a stale async emit is dropped
    /// before it serializes the row tree.
    pub(super) fn emit_queue(&mut self, force: bool) {
        let revision = self.player.revision();
        if !force && !self.queue_rows_dirty && self.queue_revision == Some(revision) {
            return;
        }
        let invoker = self.get_qml_method_invoker();
        let Some(dbs) = crate::app::dbs() else {
            return;
        };
        // Record the revision only once the emit is actually going out: a
        // missing DB must leave the next call free to retry.
        self.queue_revision = Some(revision);
        self.queue_rows_dirty = false;
        let queue = self.player.queue_snapshot();
        let index = self.player.index();
        let generation = QUEUE_EMIT_GEN.fetch_add(1, Ordering::Relaxed) + 1;
        crate::app::runtime().spawn(async move {
            let starred: std::collections::HashSet<String> =
                crate::library::starred_song_ids(&dbs.library)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
            // A newer emit already went out: this one is stale. Check before
            // the serialization, not after it: a stale pass used to build the
            // whole row tree and only then discover it had lost.
            if QUEUE_EMIT_GEN.load(Ordering::Relaxed) != generation {
                return;
            }
            // Stream the row set once: the queue can hold the entire library,
            // and this string doubles as the dedupe hash input.
            let _row_count = queue.len();
            let rows = QueueRows {
                queue: queue.as_slice(),
                starred: &starred,
            };
            let Ok(rows_json) = serde_json::to_string(&rows) else {
                eprintln!("queue: could not serialize the queue rows");
                return;
            };
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&rows_json, &mut hasher);
            let rows_hash = std::hash::Hasher::finish(&hasher);
            // QR-3: swap unconditionally — a forced publish must store its
            // hash too, or the next automatic emit of the same rows
            // republishes against a stale value.
            let previous = QUEUE_ROWS_HASH.swap(rows_hash, Ordering::Relaxed);
            if !force && previous == rows_hash {
                return;
            }
            crate::debug_log!("queue: published {_row_count} rows");
            // The rows were just serialized for the hash; splice that string
            // into the envelope instead of serializing the whole row set a
            // second time (the queue can hold the entire library).
            let json = format!(
                r#"{{"index":{},"items":{},"hash":"{}"}}"#,
                index, rows_json, rows_hash
            );
            emit(&invoker, "queue", json);
        });
    }

    /// Publishes changed scalar player fields; the 500 ms tick compares first
    /// so unchanged strings are never re-emitted.
    pub(super) fn sync_player(&mut self) {
        // ARC-10: only emit when a value actually changed — the 500 ms tick
        // must not republish every string on every frame. Compare first (no
        // clones), clone only the fields that changed.
        macro_rules! sync_current {
            ($field:ident, $signal:ident, $field_name:ident) => {{
                let value = self
                    .player
                    .current()
                    .map(|track| track.$field_name.as_str())
                    .unwrap_or_default();
                let changed = self.$field.as_str() != value;
                if changed {
                    self.$field = self
                        .player
                        .current()
                        .map(|track| track.$field_name.clone())
                        .unwrap_or_default();
                    self.$signal();
                }
                changed
            }};
        }

        if sync_current!(current_song_id, current_song_id_changed, id) {
            // P-6 S2: starts, skips and auto-advance all count as "last
            // song"; clearing the track keeps the previous one.
            let song_id = self.current_song_id.clone();
            self.remember_song(&song_id);
        }
        sync_current!(current_album_id, current_album_id_changed, album_id);
        sync_current!(current_title, current_title_changed, title);
        sync_current!(current_artist, current_artist_changed, artist);
        sync_current!(current_album, current_album_changed, album);
        sync_current!(current_cover_art, current_cover_art_changed, cover_art);
        sync_current!(current_cover_url, current_cover_url_changed, cover_url);

        macro_rules! sync_field {
            ($field:ident, $signal:ident, $value:expr) => {{
                let value = $value;
                if self.$field != value {
                    self.$field = value;
                    self.$signal();
                }
            }};
        }

        sync_field!(
            playback_state,
            playback_state_changed,
            self.player.state().name().to_string()
        );
        sync_field!(duration, duration_changed, self.player.duration());
        sync_field!(position, position_changed, self.player.position());
        sync_field!(volume, volume_changed, self.player.volume());
        sync_field!(muted, muted_changed, self.player.muted());
        sync_field!(queue_index, queue_index_changed, self.player.index() as i64);
        sync_field!(queue_length, queue_length_changed, self.player.len() as i64);
        sync_field!(
            current_song_starred,
            current_song_starred_changed,
            self.player.current().map(|track| track.favorite).unwrap_or(false)
        );

        // Q-8: mirror the player into the native MPRIS service.
        crate::mpris::publish(self.mpris_snapshot());
    }
}
