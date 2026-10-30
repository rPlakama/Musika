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

//! The global row-cache budget shared by every list surface.
//!
//! Surfaces keep only the rows around their visible window here; the full
//! dataset count stays with the surface's virtual model. This module owns the
//! single 120-row ceiling and the LRU/pinning policy. It is deliberately
//! independent of Qt so the policy is unit-testable before the virtual models
//! are wired to it.

use std::collections::{HashMap, HashSet, VecDeque};

use qtbridge::{QModelItem, qobject};

/// Hard ceiling for row records resident across every list surface.
pub const MAX_RESIDENT_ROWS: usize = 120;

/// Maximum rows any one surface may hold around its viewport.
pub const LIVE_WINDOW_ROWS: usize = 30;

/// The list surface a cached row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Albums,
    Songs,
    Artists,
    AlbumTracks,
    ArtistAlbums,
    Queue,
    Search,
    Lyrics,
}

/// One row's position in a surface dataset.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub surface: Surface,
    /// Bumped whenever the surface's filter, order or dataset changes.
    pub generation: u64,
    pub index: usize,
}

impl RowKey {
    pub fn new(surface: Surface, generation: u64, index: usize) -> Self {
        Self {
            surface,
            generation,
            index,
        }
    }
}

/// The common row payload shared by the virtual QML models.
#[derive(Clone, Debug, Default, PartialEq, Eq, QModelItem, serde::Serialize)]
#[allow(non_snake_case)]
pub struct RowRecord {
    pub songId: String,
    pub albumId: String,
    pub artistId: String,
    pub artist: String,
    pub artistCover: String,
    pub year: i32,
    pub tracks: i32,
    pub cover: String,
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub track: i32,
    pub duration: i32,
    pub starred: bool,
    pub rating: i32,
}

/// A bounded, pin-aware LRU of row records.
#[derive(Debug)]
pub struct RowCache {
    max: usize,
    entries: HashMap<RowKey, RowRecord>,
    order: VecDeque<RowKey>,
    pinned: HashSet<RowKey>,
}

impl RowCache {
    pub fn new(max: usize) -> Self {
        Self {
            max,
            entries: HashMap::new(),
            order: VecDeque::new(),
            pinned: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn get(&self, key: &RowKey) -> Option<&RowRecord> {
        self.entries.get(key)
    }

    /// Inserts or refreshes a row and evicts the oldest unpinned rows until
    /// the budget is respected. Returns true if any row was evicted.
    pub fn insert(&mut self, key: RowKey, row: RowRecord) -> bool {
        self.order.retain(|existing| existing != &key);
        self.order.push_back(key.clone());
        self.entries.insert(key, row);
        self.evict()
    }

    pub fn pin(&mut self, key: RowKey) {
        self.pinned.insert(key);
    }

    pub fn unpin(&mut self, key: &RowKey) {
        self.pinned.remove(key);
    }

    pub fn clear_surface(&mut self, surface: Surface) {
        self.entries.retain(|key, _| key.surface != surface);
        self.order.retain(|key| key.surface != surface);
        self.pinned.retain(|key| key.surface != surface);
    }

    pub fn clear_generation(&mut self, surface: Surface, generation: u64) {
        self.entries
            .retain(|key, _| key.surface != surface || key.generation == generation);
        self.order
            .retain(|key| key.surface != surface || key.generation == generation);
        self.pinned
            .retain(|key| key.surface != surface || key.generation == generation);
    }

    fn evict(&mut self) -> bool {
        let mut evicted = false;
        while self.entries.len() > self.max {
            let Some(position) = self.order.iter().position(|key| !self.pinned.contains(key))
            else {
                // Every resident row is pinned: temporarily exceed the cap
                // rather than blanking the visible UI. The caller can log it.
                break;
            };
            let Some(key) = self.order.remove(position) else {
                break;
            };
            self.entries.remove(&key);
            evicted = true;
        }
        evicted
    }
}

impl Default for RowCache {
    fn default() -> Self {
        Self::new(MAX_RESIDENT_ROWS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(index: usize) -> RowKey {
        RowKey::new(Surface::Songs, 1, index)
    }

    fn row(index: usize) -> RowRecord {
        RowRecord {
            songId: format!("s{index}"),
            ..RowRecord::default()
        }
    }

    #[test]
    fn enforces_the_global_cap_and_evicts_oldest() {
        let mut cache = RowCache::new(3);
        assert!(!cache.insert(key(0), row(0)));
        assert!(!cache.insert(key(1), row(1)));
        assert!(!cache.insert(key(2), row(2)));
        assert!(cache.insert(key(3), row(3)));
        assert_eq!(cache.len(), 3);
        assert!(cache.get(&key(0)).is_none());
        assert!(cache.get(&key(3)).is_some());
    }

    #[test]
    fn pinned_rows_survive_eviction() {
        let mut cache = RowCache::new(2);
        cache.insert(key(0), row(0));
        cache.insert(key(1), row(1));
        cache.pin(key(0));
        cache.insert(key(2), row(2));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&key(0)).is_some());
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some());
    }

    #[test]
    fn all_pinned_rows_temporarily_exceed_the_cap() {
        let mut cache = RowCache::new(2);
        cache.insert(key(0), row(0));
        cache.insert(key(1), row(1));
        cache.pin(key(0));
        cache.pin(key(1));
        cache.pin(key(2));
        cache.insert(key(2), row(2));
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn clear_generation_drops_only_the_old_dataset() {
        let mut cache = RowCache::new(4);
        cache.insert(RowKey::new(Surface::Songs, 1, 0), row(0));
        cache.insert(RowKey::new(Surface::Songs, 2, 0), row(1));
        cache.insert(RowKey::new(Surface::Albums, 1, 0), row(2));
        cache.clear_generation(Surface::Songs, 2);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&RowKey::new(Surface::Songs, 1, 0)).is_none());
        assert!(cache.get(&RowKey::new(Surface::Songs, 2, 0)).is_some());
        assert!(cache.get(&RowKey::new(Surface::Albums, 1, 0)).is_some());
    }
}

/// The first virtual list surface. Only songs are wired in this phase; the
/// remaining surfaces reuse this model once their range providers land.
#[qobject(Base = QListModel)]
pub mod row_model {
    use std::collections::HashMap;

    use qtbridge::{QListModel, QListModelBase};

    use super::{LIVE_WINDOW_ROWS, RowRecord};

    #[derive(Default)]
    pub struct RowModel {
        surface: String,
        order: String,
        filter: String,
        count: usize,
        rows: HashMap<usize, RowRecord>,
    }

    impl QListModel for RowModel {
        type Item = RowRecord;

        fn len(&self) -> usize {
            self.count
        }

        fn get(&self, index: usize) -> Option<&Self::Item> {
            self.rows.get(&index)
        }

        fn set_unnotified(&mut self, index: usize, value: Self::Item) -> bool {
            self.rows.insert(index, value);
            true
        }

        fn reset_unnotified(&mut self) {}
    }

    impl RowModel {
        #[qslot(qml_name = "setSurface")]
        pub fn set_surface(&mut self, surface: String) {
            if self.surface != surface {
                self.surface = surface;
                self.count = 0;
                self.rows.clear();
                self.reset();
            }
        }

        #[qslot(qml_name = "setOrder")]
        pub fn set_order(&mut self, order: String) {
            if self.order != order {
                self.order = order;
                self.rows.clear();
                self.reset();
            }
        }

        #[qslot(qml_name = "setFilter")]
        pub fn set_filter(&mut self, filter: String) {
            if self.filter != filter {
                self.filter = filter;
                self.rows.clear();
                self.reset();
            }
        }

        /// Loads the requested inclusive index range into the sparse model.
        /// Synchronous for this first surface: the window is bounded to 30
        /// rows, so the SQLite read is short and the GUI never waits on the
        /// whole library.
        #[qslot(qml_name = "ensure")]
        pub fn ensure(&mut self, start: i64, end: i64) {
            match self.surface.as_str() {
                "songs" => self.ensure_songs(start, end),
                "albums" => self.ensure_albums(start, end),
                "artists" => self.ensure_artists(start, end),
                "artist_albums" => self.ensure_artist_albums(start, end),
                _ => {}
            }
        }

        fn prepare_window(
            &self,
            count: usize,
            start: i64,
            end: i64,
        ) -> Option<(usize, usize, bool)> {
            if count == 0 {
                return None;
            }
            let changed = count != self.count;
            let start = start.max(0) as usize;
            let end = end
                .max(start as i64)
                .min(start as i64 + LIVE_WINDOW_ROWS as i64)
                .min(count as i64 - 1) as usize;
            (end >= start).then_some((start, end, changed))
        }

        fn finish_window(
            &mut self,
            count: usize,
            reset: bool,
            records: Vec<(usize, RowRecord)>,
        ) {
            if reset {
                self.count = count;
                self.rows = records.into_iter().collect();
                self.reset();
            } else {
                for (index, row) in records {
                    self.set(index, row);
                }
            }
        }

        fn ensure_songs(&mut self, start: i64, end: i64) {
            let Some(dbs) = crate::app::dbs() else { return };
            let runtime = crate::app::runtime();
            let count = runtime.block_on(crate::library::song_count(&dbs.library)).unwrap_or(0);
            let Some((start, end, reset)) = self.prepare_window(count, start, end) else { return };
            let order = if self.order.is_empty() { "Title".to_string() } else { self.order.clone() };
            if !reset {
                self.rows.retain(|index, _| *index >= start && *index <= end);
            }
            let rows = runtime.block_on(crate::library::songs_range(
                &dbs.library, start as i64, (end - start + 1) as i64, &order,
            )).unwrap_or_default();
            let covers = runtime.block_on(crate::covers::url_map_for(
                &dbs, rows.iter().map(|row| &row.cover_art),
            )).unwrap_or_default();
            let mut records = Vec::new();
            for (offset, row) in rows.into_iter().enumerate() {
                let record = RowRecord {
                    songId: row.id.clone(),
                    albumId: row.album_id.clone().unwrap_or_default(),
                    cover: row.cover_art.as_ref().and_then(|id| covers.get(id).cloned()).unwrap_or_default(),
                    title: row.title,
                    subtitle: row.artist.unwrap_or_default(),
                    meta: crate::browse::rows::format_duration(row.duration),
                    track: row.track.unwrap_or(0),
                    duration: row.duration.unwrap_or(0),
                    starred: row.starred.is_some(),
                    ..RowRecord::default()
                };
                records.push((start + offset, record));
            }
            self.finish_window(count, reset, records);
        }

        fn ensure_albums(&mut self, start: i64, end: i64) {
            let Some(dbs) = crate::app::dbs() else { return };
            let runtime = crate::app::runtime();
            let artist = (!self.filter.is_empty()).then(|| self.filter.clone());
            let count = runtime
                .block_on(crate::library::album_count(&dbs.library, artist.as_deref()))
                .unwrap_or(0);
            let Some((start, end, reset)) = self.prepare_window(count, start, end) else { return };
            let order = if self.order.is_empty() { "Artist".to_string() } else { self.order.clone() };
            if !reset {
                self.rows.retain(|index, _| *index >= start && *index <= end);
            }
            let rows = runtime.block_on(crate::library::albums_range(
                &dbs.library, artist.as_deref(), &order, start as i64, (end - start + 1) as i64,
            )).unwrap_or_default();
            let covers = runtime.block_on(crate::covers::url_map_for(
                &dbs, rows.iter().map(|row| &row.cover_art),
            )).unwrap_or_default();
            let mut records = Vec::new();
            for (offset, row) in rows.into_iter().enumerate() {
                let cover = row.cover_art.as_ref().and_then(|id| covers.get(id).cloned()).unwrap_or_default();
                let subtitle = match row.year {
                    Some(year) => format!("{} · {year}", row.artist.as_deref().unwrap_or("")),
                    None => row.artist.clone().unwrap_or_default(),
                };
                let record = RowRecord {
                    albumId: row.id,
                    cover,
                    title: row.title,
                    artist: row.artist.clone().unwrap_or_default(),
                    subtitle,
                    meta: format!("{} track{}", row.song_count, if row.song_count == 1 { "" } else { "s" }),
                    tracks: row.song_count as i32,
                    year: row.year.unwrap_or(0),
                    starred: row.starred.is_some(),
                    rating: row.rating as i32,
                    ..RowRecord::default()
                };
                records.push((start + offset, record));
            }
            self.finish_window(count, reset, records);
        }

        fn ensure_artists(&mut self, start: i64, end: i64) {
            let Some(dbs) = crate::app::dbs() else { return };
            let runtime = crate::app::runtime();
            let count = runtime.block_on(crate::library::artist_count(&dbs.library)).unwrap_or(0);
            let Some((start, end, reset)) = self.prepare_window(count, start, end) else { return };
            let order = if self.order.is_empty() { "Name".to_string() } else { self.order.clone() };
            if !reset {
                self.rows.retain(|index, _| *index >= start && *index <= end);
            }
            let rows = runtime.block_on(crate::library::artists_range(
                &dbs.library, &order, start as i64, (end - start + 1) as i64,
            )).unwrap_or_default();
            let covers = runtime.block_on(crate::covers::url_map_for(
                &dbs, rows.iter().map(|row| &row.cover_art),
            )).unwrap_or_default();
            let starred = runtime.block_on(crate::artist_user::starred_ids(&dbs.artist_user)).unwrap_or_default();
            let ratings = runtime.block_on(crate::artist_user::ratings(&dbs.artist_user)).unwrap_or_default();
            let mut records = Vec::new();
            for (offset, row) in rows.into_iter().enumerate() {
                let cover = row.cover_art.as_ref().and_then(|id| covers.get(id).cloned()).unwrap_or_default();
                let record = RowRecord {
                    artistId: row.id.clone(),
                    artist: row.name.clone(),
                    artistCover: cover.clone(),
                    cover,
                    title: row.name,
                    meta: format!("{} album{}", row.album_count, if row.album_count == 1 { "" } else { "s" }),
                    tracks: row.album_count as i32,
                    starred: row.starred.is_some() || starred.contains(&row.id),
                    rating: ratings.get(&row.id).copied().unwrap_or(0) as i32,
                    ..RowRecord::default()
                };
                records.push((start + offset, record));
            }
            self.finish_window(count, reset, records);
        }

        fn ensure_artist_albums(&mut self, start: i64, end: i64) {
            let Some(dbs) = crate::app::dbs() else { return };
            let runtime = crate::app::runtime();
            let artist_id = self.filter.clone();
            let count = runtime
                .block_on(crate::library::artist_album_count(&dbs.library, &artist_id))
                .unwrap_or(0);
            let Some((start, end, reset)) = self.prepare_window(count, start, end) else { return };
            let order = if self.order.is_empty() { "Year".to_string() } else { self.order.clone() };
            if !reset {
                self.rows.retain(|index, _| *index >= start && *index <= end);
            }
            let rows = runtime
                .block_on(crate::library::artist_albums_range(
                    &dbs.library,
                    &artist_id,
                    &order,
                    start as i64,
                    (end - start + 1) as i64,
                ))
                .unwrap_or_default();
            let covers = runtime
                .block_on(crate::covers::url_map_for(
                    &dbs,
                    rows.iter().map(|row| &row.cover_art),
                ))
                .unwrap_or_default();
            let mut records = Vec::new();
            for (offset, row) in rows.into_iter().enumerate() {
                let cover = row
                    .cover_art
                    .as_ref()
                    .and_then(|id| covers.get(id).cloned())
                    .unwrap_or_default();
                let subtitle = match row.year {
                    Some(year) => format!("{} · {year}", row.artist.as_deref().unwrap_or("")),
                    None => row.artist.clone().unwrap_or_default(),
                };
                let record = RowRecord {
                    albumId: row.id,
                    artist: row.artist.clone().unwrap_or_default(),
                    cover,
                    title: row.title,
                    subtitle,
                    meta: format!("{} track{}", row.song_count, if row.song_count == 1 { "" } else { "s" }),
                    tracks: row.song_count as i32,
                    year: row.year.unwrap_or(0),
                    starred: row.starred.is_some(),
                    rating: row.rating as i32,
                    ..RowRecord::default()
                };
                records.push((start + offset, record));
            }
        }


        #[qslot(qml_name = "row")]
        pub fn row_json(&self, index: i64) -> String {
            let index = index.max(0) as usize;
            self.rows
                .get(&index)
                .and_then(|row| serde_json::to_string(row).ok())
                .unwrap_or_else(|| "null".to_string())
        }
    }
}
