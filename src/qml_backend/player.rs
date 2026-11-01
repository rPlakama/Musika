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

use qtbridge::QmlObject;

use super::{Backend, QueueCoversEnvelope, emit};

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

    /// Destructive maintenance (the SuperiorBar trashcan's quadrant menu —
    /// a click or Alt while it is scoped): `scope` is "all", "library",
    /// "indexes" or "covers". The databases are dropped and re-migrated from
    /// the embedded schema; "all" also removes credentials and returns to the
    /// Login screen.
    pub(super) fn emit_queue(&mut self, force: bool) {
        let revision = self.player.revision();
        if !force && !self.queue_rows_dirty && self.queue_revision == Some(revision) {
            return;
        }
        let invoker = self.get_qml_method_invoker();
        // Queue rows are virtual: publish only the queue identity. The drawer
        // asks for its visible window through `requestQueueWindow`, so a
        // library-wide queue no longer serializes thousands of rows.
        self.queue_revision = Some(revision);
        self.queue_rows_dirty = false;
        let queue = self.player.queue_snapshot();
        let json = serde_json::json!({
            "index": self.player.index(),
            "count": queue.len(),
            "hash": revision,
        })
        .to_string();
        crate::debug_log!("queue: published {} rows", queue.len());
        emit(&invoker, "queue", json);
    }

    /// Re-reads the download ledger for the requested window (`songs` and
    /// `albums` are the JSON arrays of the currently visible rows) — the row
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
