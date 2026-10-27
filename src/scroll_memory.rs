// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

//! UI-54 scroll memory, owned by Rust.
//!
//! The QML surfaces only ask for an entry or remember one; TTL pruning, the
//! entry cap, startup-row-zero policy and the persisted JSON shape live here.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const SCROLL_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const SCROLL_MAX_ENTRIES: usize = 200;
const STARTUP_ROW_ZERO: [&str; 4] = ["albums", "songs", "artists", "playlists"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ScrollEntry {
    #[serde(default = "minus_one")]
    pub index: i64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "minus_one")]
    pub scope: i64,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub saved_at: u64,
}

fn minus_one() -> i64 {
    -1
}

#[derive(Default)]
struct Store {
    hydrated: bool,
    entries: HashMap<String, ScrollEntry>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

fn store() -> MutexGuard<'static, Store> {
    STORE
        .get_or_init(|| Mutex::new(Store::default()))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn startup_zero(key: &str) -> bool {
    STARTUP_ROW_ZERO.contains(&key)
}

impl Store {
    fn prune(&mut self) {
        let current = now_ms();
        self.entries.retain(|_, entry| {
            entry.saved_at == 0 || current.saturating_sub(entry.saved_at) <= SCROLL_TTL_MS
        });
        if self.entries.len() <= SCROLL_MAX_ENTRIES {
            return;
        }
        let mut keys: Vec<String> = self.entries.keys().cloned().collect();
        keys.sort_by_key(|key| self.entries.get(key).map(|e| e.saved_at).unwrap_or(0));
        let remove = keys.len().saturating_sub(SCROLL_MAX_ENTRIES);
        for key in keys.into_iter().take(remove) {
            self.entries.remove(&key);
        }
    }

    fn hydrate(&mut self, raw: &str) {
        let Ok(parsed) = serde_json::from_str::<HashMap<String, ScrollEntry>>(raw) else {
            return;
        };
        let current = now_ms();
        for (key, mut entry) in parsed {
            if self.entries.contains_key(&key) {
                continue;
            }
            if !entry.y.is_finite() {
                continue;
            }
            if entry.saved_at == 0 {
                entry.saved_at = current;
            }
            if startup_zero(&key) {
                entry.index = -1;
                entry.y = 0.0;
            }
            self.entries.insert(key, entry);
        }
        self.prune();
    }

    fn serialize(&mut self) -> String {
        self.prune();
        serde_json::to_string(&self.entries).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Test-only raw hydration seam.
#[cfg(test)]
pub(crate) fn hydrate_raw(raw: &str) {
    store().hydrate(raw);
}

/// The persisted store, read once. Safe to call from every `ScrollMemory`.
pub(crate) fn hydrate() {
    let mut store = store();
    if store.hydrated {
        return;
    }
    store.hydrated = true;
    let Ok(path) = crate::db::scroll_path() else {
        return;
    };
    if let Ok(raw) = std::fs::read_to_string(path) {
        store.hydrate(&raw);
    }
}

pub(crate) fn remember(key: &str, index: i64, y: f64, scope: i64, layout: &str) {
    if key.is_empty() || index < 0 {
        return;
    }
    let mut store = store();
    store.hydrated = true;
    store.entries.insert(
        key.to_string(),
        ScrollEntry {
            index,
            y,
            scope,
            layout: layout.to_string(),
            saved_at: now_ms(),
        },
    );
}

pub(crate) fn remember_layout(key: &str, layout: &str) {
    if key.is_empty() {
        return;
    }
    let mut store = store();
    store.hydrated = true;
    let entry = store.entries.entry(key.to_string()).or_insert(ScrollEntry {
        index: -1,
        y: 0.0,
        scope: -1,
        layout: String::new(),
        saved_at: now_ms(),
    });
    entry.layout = layout.to_string();
    entry.saved_at = now_ms();
}

pub(crate) fn entry_json(key: &str) -> String {
    let store = store();
    store
        .entries
        .get(key)
        .and_then(|entry| serde_json::to_string(entry).ok())
        .unwrap_or_default()
}

pub(crate) fn saved_index(key: &str) -> i64 {
    store().entries.get(key).map(|entry| entry.index).unwrap_or(-1)
}

pub(crate) fn saved_layout(key: &str) -> String {
    store()
        .entries
        .get(key)
        .map(|entry| entry.layout.clone())
        .unwrap_or_default()
}

pub(crate) fn clear(key: &str) {
    if key.is_empty() {
        return;
    }
    store().entries.remove(key);
}

pub(crate) fn serialize() -> String {
    store().serialize()
}
