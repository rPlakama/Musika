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
//! out of the frame loop:
//!
//! - [`CoverStore::row_path`] resolves a cover URL to the file a row should
//!   paint, preferring the generated 320 px thumbnail. Successes *and*
//!   failures are cached per URL, so a second frame does not touch the disk.
//! - [`CoverStore::display_path`] resolves the display-sized copy for a
//!   `(source path, bucket)` pair. It only checks whether the copy exists;
//!   generating one is the bridge's job (on a blocking pool) and the result
//!   is published with [`CoverStore::insert_display`]. No resize ever runs
//!   inside a render pass.
//!
//! Both caches are FIFO-bounded and dropped by [`CoverStore::clear`], which
//! the bridge should call when the library or the cover cache changes.
//!
//! This module is std-only on purpose: it must not depend on GPUI, the
//! databases or the image decoder, so the path policy stays unit-testable.

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::hash::Hash;
use std::path::{Path, PathBuf};

/// Decode buckets for display-sized copies (long edge, physical px).
///
/// Generation mirrors `covers::get_or_create_resized_cover`, which writes
/// `<stem>_<edge>x<edge>.<ext>` next to the original. An edge outside this
/// list is rejected by [`DisplayKey::new`] instead of naming a file that no
/// producer would ever write.
pub const DISPLAY_BUCKETS: [u32; 4] = [256, 512, 768, 1024];

/// Chooses the smallest display bucket that covers `logical_px` at
/// `scale_factor`.
///
/// The caller must pass a fixed hero size, not a measured layout size: a
/// measured size can change between frames and make the bucket flip, which
/// would thrash the display cache and the bridge's resize jobs. A non-finite,
/// negative or zero input falls back to the smallest bucket so a broken layout
/// value can never select the original file.
pub fn bucket_for(logical_px: f32, scale_factor: f32) -> u32 {
    let smallest = DISPLAY_BUCKETS[0];
    if !logical_px.is_finite()
        || !scale_factor.is_finite()
        || logical_px <= 0.0
        || scale_factor <= 0.0
    {
        return smallest;
    }
    let need = (logical_px * scale_factor).ceil();
    DISPLAY_BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket as f32 >= need)
        .unwrap_or_else(|| *DISPLAY_BUCKETS.last().expect("display buckets are non-empty"))
}

/// The 320 px thumbnail path for `source`, mirroring `covers::thumb_name`
/// (keep the two in sync). Exposed for hero/display callers; the file is not
/// probed here.
pub fn thumb_for(source: &Path) -> Option<PathBuf> {
    thumb_path(source)
}

/// The cache key for a display copy: the source path and the target long
/// edge together identify one generated file.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DisplayKey {
    source: PathBuf,
    edge: u32,
}

impl DisplayKey {
    /// Builds a key for `source` at `edge`, or `None` when `edge` is not one
    /// of [`DISPLAY_BUCKETS`].
    ///
    /// `source` must be the exact path later passed to
    /// `covers::get_or_create_resized_cover` (the original art file, not the
    /// 320 px thumbnail): [`DisplayKey::path`] reproduces that function's
    /// file name, so the key and the producer must agree on the base path.
    pub fn new(source: impl AsRef<Path>, edge: u32) -> Option<Self> {
        if !DISPLAY_BUCKETS.contains(&edge) {
            return None;
        }
        Some(Self {
            source: source.as_ref().to_path_buf(),
            edge,
        })
    }

    /// The original art file this copy is generated from.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The bucket long edge, in physical pixels.
    pub fn edge(&self) -> u32 {
        self.edge
    }

    /// `<stem>_<edge>x<edge>.<ext>` beside the source, the same name
    /// `covers::get_or_create_resized_cover` writes.
    ///
    /// Returns `None` for a source without a parent, file stem or extension;
    /// the producer cannot name a copy for it either.
    pub fn path(&self) -> Option<PathBuf> {
        let stem = self.source.file_stem()?.to_str()?;
        let extension = self.source.extension()?.to_str()?;
        let parent = self.source.parent()?;
        let edge = self.edge;
        Some(parent.join(format!("{stem}_{edge}x{edge}.{extension}")))
    }
}

/// Bounded, path-only cover caches shared by every GPUI surface.
///
/// The store holds no decoded pixels and performs no image work: it only
/// remembers which file a URL resolves to. Both caches retain at most
/// `max_entries` resolutions in insertion order; the oldest entry is evicted
/// when a new URL/key arrives. A capacity of zero disables retention (useful
/// as a diagnostic escape hatch), so every lookup probes the disk again.
#[derive(Debug)]
pub struct CoverStore {
    rows: BoundedFifo<String, Option<PathBuf>>,
    displays: BoundedFifo<DisplayKey, Option<PathBuf>>,
    /// Cached hero fallback probes (thumbnail-or-nothing, never the original).
    heroes: BoundedFifo<DisplayKey, Option<PathBuf>>,
    /// Display keys whose resize is currently in flight in the bridge.
    pending: HashSet<DisplayKey>,
}

impl CoverStore {
    /// Creates a store retaining at most `max_entries` row URLs and
    /// `max_entries` display keys (they are separate budgets).
    pub fn new(max_entries: usize) -> Self {
        Self {
            rows: BoundedFifo::new(max_entries),
            displays: BoundedFifo::new(max_entries),
            heroes: BoundedFifo::new(max_entries),
            pending: HashSet::new(),
        }
    }

    /// Drops every cached resolution, including remembered failures.
    ///
    /// Call this after a library change, a cover sync or an account switch:
    /// cover URLs are content-addressed, but thumbnails and display copies
    /// that appeared after a failed lookup are only picked up again once
    /// their entry is gone.
    pub fn clear(&mut self) {
        self.rows.clear();
        self.displays.clear();
        self.heroes.clear();
        self.pending.clear();
    }

    /// Number of cached row resolutions (successes and failures).
    pub fn row_len(&self) -> usize {
        self.rows.count()
    }

    /// Number of cached display resolutions (successes and misses).
    pub fn display_len(&self) -> usize {
        self.displays.count()
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
    /// exists. `mod.rs::cover_path` only consulted the thumbnail for
    /// originals above 512 KiB; retaining that gate is unnecessary now that a
    /// present thumbnail is always the cheaper decode (for art at or below
    /// the 320 px edge `build_thumb` copies the original) and a missing one
    /// always falls back to the original. The original's size is therefore
    /// not re-checked.
    ///
    /// The answer is cached for the URL (fragment ignored), including `None`,
    /// so repeated frames perform no filesystem access. Falling back to the
    /// original is intentional for list rows but expensive for a full-size
    /// hero; use [`CoverStore::hero_path`] on those surfaces.
    ///
    /// A thumbnail that
    /// lands after a cached miss needs [`CoverStore::clear`], which the
    /// bridge should call when a cover sync completes. A cached success also
    /// keeps pointing at its file if the file disappears; that is the point
    /// of the cache, and fresh payloads change the content-addressed URL
    /// instead.
    pub fn row_path(&mut self, url: &str) -> Option<PathBuf> {
        let key = url_key(url)?;
        if let Some(cached) = self.rows.get(key) {
            return cached.clone();
        }
        let resolved = resolve_row_path(key);
        self.rows.insert(key.to_string(), resolved.clone());
        resolved
    }

    /// Resolves the display-sized copy for `key`, or `None` when it has not
    /// been generated yet.
    ///
    /// The first call probes the derived `<stem>_<edge>x<edge>.<ext>` once
    /// and caches the answer — including a miss — so later frames do not
    /// `stat`. This method never decodes or resizes: the bridge runs
    /// `covers::get_or_create_resized_cover` on a blocking pool and then
    /// publishes the path with [`CoverStore::insert_display`]. While the copy
    /// is missing every call returns `None`; deduplication of in-flight
    /// resize requests belongs to the bridge.
    pub fn display_path(&mut self, key: &DisplayKey) -> Option<PathBuf> {
        if let Some(cached) = self.displays.get(key) {
            return cached.clone();
        }
        let resolved = key.path().filter(|path| path.is_file());
        self.displays.insert(key.clone(), resolved.clone());
        resolved
    }

    /// Resolves the file a hero/display surface should paint.
    ///
    /// It prefers the generated display copy, then the 320 px thumbnail, and
    /// otherwise returns `None`. It deliberately never falls back to the
    /// original: a 4000 px cover is tens of MB of decoded RGBA and must not be
    /// uploaded or downscaled inside a frame.
    ///
    /// Both the display probe and the thumbnail probe are cached per key, so
    /// a missing bucket does not `stat` on every frame. The original is never
    /// a candidate here.
    pub fn hero_path(&mut self, key: &DisplayKey) -> Option<PathBuf> {
        if let Some(display) = self.display_path(key) {
            return Some(display);
        }
        if let Some(cached) = self.heroes.get(key) {
            return cached.clone();
        }
        let thumb = thumb_for(key.source()).filter(|path| path.is_file());
        self.heroes.insert(key.clone(), thumb.clone());
        thumb
    }

    /// Starts a resize request for `key`, returning `true` only once.
    ///
    /// This module is std-only and does no decoding or dimension work: the
    /// bridge still owns "is the source already smaller than this bucket?",
    /// the blocking resize and the resulting [`CoverStore::insert_display`].
    /// This method only de-duplicates in-flight keys and skips a key whose
    /// display copy is already resolved.
    pub fn begin_display_request(&mut self, key: &DisplayKey) -> bool {
        if self.pending.contains(key) || self.displays.get(key).is_some_and(|path| path.is_some()) {
            return false;
        }
        self.pending.insert(key.clone());
        true
    }

    /// Number of display resizes currently in flight (diagnostics).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Publishes a resolved display copy (for example the path returned by
    /// `covers::get_or_create_resized_cover`), replacing a cached miss.
    ///
    /// The path is trusted as-is: producers write it atomically before
    /// inserting, so no `stat` is needed here.
    pub fn insert_display(&mut self, key: &DisplayKey, path: impl AsRef<Path>) {
        self.displays
            .insert(key.clone(), Some(path.as_ref().to_path_buf()));
        self.pending.remove(key);
    }
}

/// A hash map with FIFO eviction: inserting a new key past `max_entries`
/// removes the oldest one. Re-inserting an existing key replaces its value
/// without changing its position.
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
    /// name carries the pid, a timestamp and a counter so the parallel test
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

        /// Creates a file of `bytes` length (sparse: content is irrelevant to
        /// path resolution).
        fn file(&self, name: &str, bytes: u64) -> PathBuf {
            let path = self.path.join(name);
            let handle = fs::File::create(&path).expect("create temporary cover");
            handle.set_len(bytes).expect("size temporary cover");
            path
        }
    }

    impl Drop for TempCoverDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn file_url(path: &Path) -> String {
        format!("file://{}", path.display())
    }

    #[test]
    fn prefers_an_existing_thumbnail() {
        let dir = TempCoverDir::new();
        let small = dir.file("small.jpg", 8 * 1024);
        let small_thumb = dir.file("small.thumb.jpg", 1024);
        let large = dir.file("large.jpg", 600 * 1024);
        let large_thumb = dir.file("large.thumb.jpg", 1024);
        let plain = dir.file("plain.png", 600 * 1024);

        let mut store = CoverStore::new(16);
        assert_eq!(store.row_path(&file_url(&small)), Some(small_thumb.clone()));
        assert_eq!(store.row_path(&file_url(&large)), Some(large_thumb.clone()));
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
    fn failed_lookup_is_not_retried_until_clear() {
        let dir = TempCoverDir::new();
        let path = dir.path().join("late.jpg");
        let url = file_url(&path);

        let mut store = CoverStore::new(8);
        assert_eq!(store.row_path(&url), None);

        // The file appearing later must not be picked up by a cached failure.
        dir.file("late.jpg", 4 * 1024);
        assert_eq!(store.row_path(&url), None, "a cached failure must not stat");
        assert_eq!(store.row_len(), 1);

        store.clear();
        assert_eq!(store.row_path(&url), Some(path));
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
    fn display_cache_separates_buckets_for_one_source() {
        let dir = TempCoverDir::new();
        let source = dir.file("raw-abc.jpg", 600 * 1024);
        let key256 = DisplayKey::new(&source, 256).expect("256 is a display bucket");
        let key512 = DisplayKey::new(&source, 512).expect("512 is a display bucket");

        assert_ne!(key256, key512);
        assert_ne!(key256.path(), key512.path());
        assert_eq!(DisplayKey::new(&source, 320), None, "unsupported bucket");
        assert_eq!(DisplayKey::new(&source, 0), None);

        let copy512 = dir.file("raw-abc_512x512.jpg", 4096);
        assert_eq!(key512.path(), Some(copy512.clone()));

        let mut store = CoverStore::new(8);
        assert_eq!(store.display_path(&key256), None);
        assert_eq!(store.display_path(&key512), Some(copy512.clone()));
        assert_eq!(
            store.display_path(&key256),
            None,
            "the 512 copy must not resolve the 256 key"
        );

        // The producer publishes a copy once the background resize finishes.
        let copy256 = dir.file("raw-abc_256x256.jpg", 4096);
        store.insert_display(&key256, &copy256);
        assert_eq!(store.display_path(&key256), Some(copy256.clone()));
        assert_eq!(store.display_len(), 2);

        // clear() invalidates display entries too; the copy now on disk is
        // found again by the lazy probe.
        store.clear();
        assert_eq!(store.display_len(), 0);
        assert_eq!(store.display_path(&key256), Some(copy256));
    }

    #[test]
    fn row_urls_strip_fragments_and_decode_escapes() {
        let dir = TempCoverDir::new();
        let path = dir.file("with space.jpg", 2048);
        let base = format!("file://{}/with%20space.jpg", dir.path().display());

        let mut store = CoverStore::new(8);
        assert_eq!(
            store.row_path(&format!("{base}#no-thumb")),
            Some(path.clone())
        );
        assert_eq!(store.row_len(), 1, "the fragment is not part of the key");
        assert_eq!(store.row_path(&base), Some(path));

        // A remote URL is not a local file; it is remembered like any other
        // failure and never re-parsed.
        assert_eq!(store.row_path("https://example.com/cover.jpg"), None);
        assert_eq!(store.row_len(), 2);
        assert_eq!(store.row_path(""), None);
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
        assert_eq!(store.row_path(&url), Some(path));
    }

    #[test]
    fn bucket_for_respects_boundaries_scale_and_bad_input() {
        assert_eq!(bucket_for(256.0, 1.0), 256);
        assert_eq!(bucket_for(257.0, 1.0), 512);
        assert_eq!(bucket_for(1024.0, 1.0), 1024);
        assert_eq!(bucket_for(5000.0, 1.0), 1024);
        assert_eq!(bucket_for(200.0, 1.5), 512);
        assert_eq!(bucket_for(256.0, 2.0), 512);
        assert_eq!(bucket_for(1.0, 2.0), 256);
        assert_eq!(bucket_for(f32::NAN, 1.0), 256);
        assert_eq!(bucket_for(256.0, f32::NAN), 256);
        assert_eq!(bucket_for(-10.0, 1.0), 256);
        assert_eq!(bucket_for(256.0, 0.0), 256);
        assert_eq!(bucket_for(0.0, 1.0), 256);
    }

    #[test]
    fn hero_path_prefers_bucket_then_thumb_and_never_original() {
        let dir = TempCoverDir::new();
        let source = dir.file("hero.jpg", 600 * 1024);
        let key = DisplayKey::new(&source, 512).expect("512 bucket");
        let mut store = CoverStore::new(8);

        // No bucket, no thumb: the hero must not fall back to the original.
        assert_eq!(store.hero_path(&key), None);

        // A thumbnail appears: the cached miss still wins until clear().
        let thumb = dir.file("hero.thumb.jpg", 2048);
        assert_eq!(store.hero_path(&key), None);
        store.clear();
        assert_eq!(store.hero_path(&key), Some(thumb.clone()));

        // A published display copy wins over the thumbnail.
        let bucket = dir.file("hero_512x512.jpg", 4096);
        store.insert_display(&key, &bucket);
        assert_eq!(store.hero_path(&key), Some(bucket));
    }

    #[test]
    fn hero_path_uses_bucket_copy_when_present() {
        let dir = TempCoverDir::new();
        let source = dir.file("raw.jpg", 1024);
        let copy = dir.file("raw_256x256.jpg", 4096);
        let key = DisplayKey::new(&source, 256).expect("256 bucket");
        let mut store = CoverStore::new(8);
        assert_eq!(store.hero_path(&key), Some(copy));
    }

    #[test]
    fn begin_display_request_dedupes_until_resolved_or_cleared() {
        let dir = TempCoverDir::new();
        let source = dir.file("pending.jpg", 1024);
        let key = DisplayKey::new(&source, 256).expect("256 bucket");
        let mut store = CoverStore::new(8);
        assert!(store.begin_display_request(&key));
        assert!(!store.begin_display_request(&key));
        assert_eq!(store.pending_len(), 1);

        let copy = dir.file("pending_256x256.jpg", 2048);
        store.insert_display(&key, &copy);
        assert_eq!(store.pending_len(), 0);
        assert!(!store.begin_display_request(&key));

        store.clear();
        assert!(store.begin_display_request(&key));
    }
}
