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

/// Hard ceiling for row records resident across every list surface.
pub const MAX_RESIDENT_ROWS: usize = 120;

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowRecord {
    pub id: String,
    pub secondary_id: String,
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
            id: format!("s{index}"),
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
