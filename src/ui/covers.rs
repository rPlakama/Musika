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

//! Cover path resolution for the GPUI front end.
//!
//! The GPUI list/hero code resolved cover URLs on every frame: parse the URL,
//! `stat` the original to decide whether it was worth thumbnailing, then
//! `stat` the derived `.thumb.jpg`. A re-rendered 200-row list could pay
//! hundreds of filesystem probes per frame. [`CoverStore`] keeps that work
//! out of the frame loop with two bounded URL→path caches:
//!
//! - [`CoverStore::row_path`] resolves the file a list row paints, preferring
//!   the generated 320 px thumbnail (rows are small; the thumbnail is the
//!   cheaper decode).
//! - [`CoverStore::original_path`] resolves the untouched provider art that
//!   hero surfaces paint (album/artist page and fullscreen cover). Heroes are
//!   scaled to hundreds of pixels, so a thumbnail or a resized display copy
//!   would visibly soften the image; the original file is the only source
//!   that is not shortened.
//!
//! The caches are std-only on purpose: this module must not depend on GPUI,
//! the databases or the image decoder, so the path policy stays unit-testable.

use std::borrow::Borrow;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::hash::Hash;
use std::path::{Path, PathBuf};

/// Bounded, path-only cover caches shared by every GPUI surface.
///
/// The store holds no decoded pixels and performs no image work: it only
/// remembers which file a URL resolves to. Both caches retain at most
/// `max_entries` resolutions in insertion order; the oldest entry is evicted
/// when a new URL arrives. A capacity of zero disables retention (useful as a
/// diagnostic escape hatch), so every lookup probes the disk again.
#[derive(Debug)]
pub struct CoverStore {
    rows: BoundedFifo<String, Option<PathBuf>>,
    originals: BoundedFifo<String, Option<PathBuf>>,
}

impl CoverStore {
    /// Creates a store retaining at most `max_entries` row resolutions and
    /// `max_entries` original resolutions (they are separate budgets).
    pub fn new(max_entries: usize) -> Self {
        Self {
            rows: BoundedFifo::new(max_entries),
            originals: BoundedFifo::new(max_entries),
        }
    }

    /// Drops every cached resolution, including remembered failures.
    ///
    /// Call this after a library change, a cover sync or an account switch:
    /// cover URLs are content-addressed, but a thumbnail or original that
    /// appeared after a failed lookup is only picked up again once its entry
    /// is gone.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.originals.clear();
    }

    /// Number of cached row resolutions (successes and failures).
    pub fn row_len(&self) -> usize {
        self.rows.count()
    }

    /// Number of cached original resolutions (successes and failures).
    pub fn original_len(&self) -> usize {
        self.originals.count()
    }

    /// Resolves the art file a row should paint for `url`.
    ///
    /// `url` is the `file://` URL produced by `covers::url_map_of`; the
    /// `#fragment` (`#no-thumb` is appended by the cover cache) is stripped
    /// before parsing. `http(s)` URLs resolve to `None`: only the bridge can
    /// turn one into a local file, and a later payload will carry the
    /// resulting `file://` URL.
    ///
    /// A generated `<stem>.thumb.jpg` beside the source wins whenever it
    /// exists; a missing thumbnail falls back to the original. The answer is
    /// cached for the URL (fragment ignored), including `None`, so repeated
    /// frames perform no filesystem access. Row covers are small, so the
    /// 320 px thumbnail is the cheaper decode; hero surfaces use
    /// [`CoverStore::original_path`] instead.
    pub fn row_path(&mut self, url: &str) -> Option<PathBuf> {
        let key = url_key(url)?;
        if let Some(cached) = self.rows.get(key) {
            return cached.clone();
        }
        let resolved = resolve_row_path(key);
        self.rows.insert(key.to_string(), resolved.clone());
        resolved
    }

    /// Resolves the untouched provider art a hero should paint for `url`.
    ///
    /// The page and fullscreen covers are scaled to hundreds of logical pixels;
    /// a 320 px thumbnail or a resized display copy is visibly softened at that
    /// size, so this deliberately never substitutes a derivative. The path is
    /// the original file itself, and the image cache that bounds decoded
    /// pixels lives in `ui::image_cache`, not here.
    ///
    /// The answer is cached per URL (fragment ignored), including a miss, so a
    /// frame loop performs one `stat` at most. `http(s)` URLs resolve to
    /// `None`: the provider URL is the streaming fallback the caller keeps.
    pub fn original_path(&mut self, url: &str) -> Option<PathBuf> {
        let key = url_key(url)?;
        if let Some(cached) = self.originals.get(key) {
            return cached.clone();
        }
        let resolved = file_url_to_path(key).filter(|path| path.is_file());
        self.originals.insert(key.to_string(), resolved.clone());
        resolved
    }
}

#[derive(Debug)]
struct BoundedFifo<K, V> {
    max_entries: usize,
    entries: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Eq + Hash + Clone, V> BoundedFifo<K, V> {
    fn new(max_entries: usize) -> Self {
        Self {
            max_entries,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Eq + Hash + ?Sized,
    {
        self.entries.get(key)
    }

    fn insert(&mut self, key: K, value: V) {
        if self.max_entries == 0 {
            return;
        }
        let is_new = self.entries.insert(key.clone(), value).is_none();
        if !is_new {
            return;
        }
        self.order.push_back(key);
        while self.entries.len() > self.max_entries {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn count(&self) -> usize {
        self.entries.len()
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

/// The cache key for a cover URL: the fragment is dropped (`#no-thumb` is a
/// hint for the producer, not part of the file name) and an empty URL is not
/// worth a cache slot.
fn url_key(url: &str) -> Option<&str> {
    let clean = url.split_once('#').map_or(url, |(head, _)| head);
    (!clean.is_empty()).then_some(clean)
}

fn resolve_row_path(url: &str) -> Option<PathBuf> {
    let source = file_url_to_path(url)?;
    if let Some(thumb) = thumb_path(&source)
        && thumb.is_file()
    {
        return Some(thumb);
    }
    let meta = std::fs::metadata(&source).ok()?;
    meta.is_file().then_some(source)
}

/// The 320 px thumbnail that would sit beside `source`, mirroring
/// `covers::thumb_name` (keep the two in sync).
fn thumb_path(source: &Path) -> Option<PathBuf> {
    let stem = source.file_stem()?.to_str()?;
    Some(source.with_file_name(format!("{stem}.thumb.jpg")))
}

/// Converts a local `file://` URL into a path without pulling in a URL crate.
///
/// The GPUI payloads are produced by `reqwest::Url::from_file_path`, so the
/// accepted shape is `file://<empty host>/abs/path` (`file://localhost/...`
/// is also accepted). Percent escapes are decoded back to bytes; a decoded
/// path that is not valid UTF-8 still survives on Unix. A remote host or a
/// non-`file` scheme yields `None`.
fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !host.is_empty() && host != "localhost" {
        return None;
    }
    Some(PathBuf::from(os_string_from_bytes(percent_decode(path))))
}

fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(decoded) = hex_pair(bytes, index) {
            out.push(decoded);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    out
}

/// Decodes `%XX` at `index`, or `None` when the escape is incomplete or
/// malformed (the `%` is then kept literally).
fn hex_pair(bytes: &[u8], index: usize) -> Option<u8> {
    if bytes.get(index) != Some(&b'%') {
        return None;
    }
    let high = hex_value(*bytes.get(index + 1)?)?;
    let low = hex_value(*bytes.get(index + 2)?)?;
    Some(high << 4 | low)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(unix)]
fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes)
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: Vec<u8>) -> OsString {
    OsString::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// A private directory under the system temp dir, removed on drop. The
    /// name carries the pid, a timestamp and a counter so parallel test
    /// threads never share one.
    struct TempCoverDir {
        path: PathBuf,
    }

    impl TempCoverDir {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default();
            let counter = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "musika-ui-covers-{}-{stamp}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temporary cover directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn file(&self, name: &str, bytes: u64) -> PathBuf {
            let path = self.path.join(name);
            let handle = fs::File::create(&path).expect("create temporary cover");
            handle.set_len(bytes).expect("size temporary cover");
            path
        }
    }

    impl Drop for TempCoverDir {
        fn drop(&mut self) {
            _ = fs::remove_dir_all(&self.path);
        }
    }

    fn file_url(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    #[test]
    fn row_path_prefers_an_existing_thumbnail() {
        let dir = TempCoverDir::new();
        let small = dir.file("small.jpg", 8 * 1024);
        let small_thumb = dir.file("small.thumb.jpg", 1024);
        let large = dir.file("large.jpg", 600 * 1024);
        let large_thumb = dir.file("large.thumb.jpg", 1024);
        let plain = dir.file("plain.png", 600 * 1024);

        let mut store = CoverStore::new(16);
        assert_eq!(store.row_path(&file_url(&small)), Some(small_thumb.clone()));
        assert_eq!(store.row_path(&file_url(&large)), Some(large_thumb));
        assert_eq!(store.row_path(&file_url(&plain)), Some(plain.clone()));

        // A cached success is served without touching the filesystem again.
        fs::remove_file(&small).expect("remove source");
        fs::remove_file(&small_thumb).expect("remove thumbnail");
        assert_eq!(store.row_path(&file_url(&small)), Some(small_thumb));

        // ... until the entry is invalidated.
        store.clear();
        assert_eq!(store.row_path(&file_url(&small)), None);
    }

    #[test]
    fn original_path_never_substitutes_a_shortened_copy() {
        let dir = TempCoverDir::new();
        let source = dir.file("hero.jpg", 1_500_000);
        let thumb = dir.file("hero.thumb.jpg", 4096);
        let display = dir.file("hero_1024x1024.jpg", 8192);
        assert!(thumb.is_file() && display.is_file());

        let mut store = CoverStore::new(16);
        assert_eq!(
            store.original_path(&file_url(&source)),
            Some(source.clone()),
            "the hero must paint the full-resolution source, not a derivative"
        );
        assert_eq!(store.row_path(&file_url(&source)), Some(thumb));
    }

    #[test]
    fn original_path_falls_back_to_the_source_without_a_thumbnail() {
        let dir = TempCoverDir::new();
        let source = dir.file("plain.jpg", 2048);
        let mut store = CoverStore::new(8);
        assert_eq!(store.row_path(&file_url(&source)), Some(source.clone()));
        assert_eq!(store.original_path(&file_url(&source)), Some(source));
    }

    #[test]
    fn failed_original_lookup_is_not_retried_until_clear() {
        let dir = TempCoverDir::new();
        let path = dir.path().join("late.jpg");
        let url = file_url(&path);

        let mut store = CoverStore::new(8);
        assert_eq!(store.original_path(&url), None);
        dir.file("late.jpg", 4 * 1024);
        assert_eq!(store.original_path(&url), None, "a cached miss must not stat");
        assert_eq!(store.original_len(), 1);

        store.clear();
        assert_eq!(store.original_path(&url), Some(path));
    }

    #[test]
    fn row_cache_evicts_in_fifo_order() {
        let dir = TempCoverDir::new();
        let paths: Vec<PathBuf> = (0..3)
            .map(|index| dir.path().join(format!("cover-{index}.jpg")))
            .collect();
        let urls: Vec<String> = paths.iter().map(|path| file_url(path)).collect();
        let mut store = CoverStore::new(2);

        assert_eq!(store.row_path(&urls[0]), None);
        assert_eq!(store.row_path(&urls[1]), None);
        assert_eq!(store.row_len(), 2);

        // Cached failures survive the files appearing later.
        dir.file("cover-0.jpg", 1024);
        dir.file("cover-1.jpg", 1024);
        assert_eq!(store.row_path(&urls[0]), None);
        assert_eq!(store.row_path(&urls[1]), None);

        // A third distinct URL evicts the oldest entry. Revisiting it probes
        // the disk again and now resolves.
        assert_eq!(store.row_path(&urls[2]), None);
        assert_eq!(store.row_len(), 2);
        assert_eq!(store.row_path(&urls[0]), Some(paths[0].clone()));
        assert_eq!(store.row_len(), 2);
    }

    #[test]
    fn original_cache_evicts_in_fifo_order_too() {
        let dir = TempCoverDir::new();
        let first = dir.file("first.jpg", 1024);
        let second = dir.file("second.jpg", 1024);
        let third = dir.file("third.jpg", 1024);
        let mut store = CoverStore::new(2);
        assert_eq!(store.original_path(&file_url(&first)), Some(first.clone()));
        assert_eq!(store.original_path(&file_url(&second)), Some(second));
        assert_eq!(store.original_len(), 2);
        assert_eq!(store.original_path(&file_url(&third)), Some(third));
        assert_eq!(store.original_len(), 2);
        // `first` was evicted; probing again finds the unchanged file.
        assert_eq!(store.original_path(&file_url(&first)), Some(first));
    }

    #[test]
    fn urls_strip_fragments_and_the_original_ignores_remote_urls() {
        let dir = TempCoverDir::new();
        let path = dir.file("with space.jpg", 2048);
        let base = format!("file://{}/with%20space.jpg", dir.path().display());

        let mut store = CoverStore::new(8);
        assert_eq!(
            store.original_path(&format!("{base}#no-thumb")),
            Some(path.clone())
        );
        assert_eq!(store.original_len(), 1, "the fragment is not part of the key");
        assert_eq!(store.original_path(&base), Some(path));
        assert_eq!(store.original_path("https://example.com/cover.jpg"), None);
        assert_eq!(store.original_len(), 2);
        assert_eq!(store.original_path(""), None);
    }

    #[cfg(unix)]
    #[test]
    fn percent_decoding_preserves_non_utf8_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let dir = TempCoverDir::new();
        let path = dir.path().join(OsStr::from_bytes(b"caf\xe9.jpg"));
        fs::write(&path, b"cover").expect("write non-UTF-8 cover");
        let url = format!("file://{}/caf%E9.jpg", dir.path().display());

        let mut store = CoverStore::new(4);
        assert_eq!(store.original_path(&url), Some(path));
    }

    #[test]
    fn zero_capacity_disables_retention() {
        let dir = TempCoverDir::new();
        let path = dir.file("no-cache.jpg", 1024);
        let mut store = CoverStore::new(0);
        assert_eq!(store.original_path(&file_url(&path)), Some(path.clone()));
        assert_eq!(store.original_len(), 0);
        assert_eq!(store.row_path(&file_url(&path)), Some(path));
        assert_eq!(store.row_len(), 0);
    }
}
