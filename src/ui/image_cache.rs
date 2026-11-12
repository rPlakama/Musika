//! Bounded LRU cache for decoded cover art.
//!
//! GPUI's [`gpui::RetainAllImageCache`] never evicts: every decoded image stays
//! in memory (and keeps its sprite-atlas texture) for the process lifetime.
//! The `img()` element without an explicit cache is worse still — its decoded
//! image is held by the app asset cache forever. Cover art is the one payload
//! that grows with every album/artist page a user visits, so entering pages
//! used to increase RSS by the decoded size of each new hero/row cover.
//!
//! [`BoundedImageCache`] implements [`gpui::ImageCache`] with two release
//! rules. An image that no element has requested for a couple of frames is
//! unloaded immediately: leaving a page (or closing an overlay) drops its
//! covers instead of keeping them resident until some later page happens to
//! need the memory. A decoded-byte budget (default 64 MiB) then backstops a
//! single screen that really does paint more than that at once. Eviction drops
//! the image from the window sprite atlas with [`gpui::App::drop_image`], so
//! neither the CPU pixels nor the texture survive. Only decoded bytes are
//! counted (not the file size): that is the number that matters for RSS.
//!
//! The cache stores no path or URL knowledge; callers hand it a `Resource`
//! through the same `img` element path every surface already uses. The one
//! app-wide cache entity is installed as the root render's image cache, so
//! every cover `img()` in the tree inherits it.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures_util::FutureExt as _;
use gpui::{
    App, AppContext, Asset, AssetLogger, Entity, ImageAssetLoader, ImageCache, ImageCacheError,
    ImageCacheItem, RenderImage, Resource, Window, hash,
};

/// Decoded RGBA budget for cover art. A 768 px hero is ~2.4 MiB, a 320 px row
/// thumbnail ~0.4 MiB; 64 MiB keeps a full hero plus several screens of row
/// covers resident without letting a long page history accumulate.
pub(crate) const DEFAULT_BUDGET_MIB: usize = 64;
const MIN_BUDGET_MIB: usize = 8;
const MAX_BUDGET_MIB: usize = 512;
const BYTES_PER_MIB: usize = 1024 * 1024;

/// Rendered frames an image may go unpainted before it is unloaded. Visible
/// covers are requested on every rendered frame (layout and paint), so two
/// frames of grace covers a page transition without re-decoding the screen it
/// just moved to; frame counting (not wall clock) means an idle window keeps
/// the cover it is showing.
const STALE_FRAMES: u64 = 2;

/// Creates the single app-wide cover cache.
///
/// `MUSIKA_IMAGE_CACHE_MB` overrides the budget (diagnostics and the memory
/// harness use it); the override is clamped so a typo cannot disable retention
/// or allocate unboundedly.
pub(crate) fn new_cache(cx: &mut App) -> Entity<BoundedImageCache> {
    let budget_mib = budget_mib();
    cx.new(|_| BoundedImageCache::new(budget_mib * BYTES_PER_MIB))
}

fn budget_mib() -> usize {
    budget_mib_from(std::env::var("MUSIKA_IMAGE_CACHE_MB").ok().as_deref())
}

/// Parses the `MUSIKA_IMAGE_CACHE_MB` value, defaulting and clamping so a
/// typo can neither disable retention nor allocate unboundedly.
fn budget_mib_from(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_BUDGET_MIB)
        .clamp(MIN_BUDGET_MIB, MAX_BUDGET_MIB)
}

/// Decoded images currently resident, and their bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
}

impl CacheStats {
    pub(crate) fn mib(self) -> f32 {
        self.bytes as f32 / BYTES_PER_MIB as f32
    }
}

/// An [`ImageCache`] that unloads images nothing paints anymore.
pub(crate) struct BoundedImageCache {
    lru: LruBudget,
    entries: HashMap<u64, Entry>,
    /// Monotonic render-frame counter. A cache entity sees one `begin_frame`
    /// per rendered frame; loads during that frame mark their image as painted.
    frame: u64,
}

struct Entry {
    item: ImageCacheItem,
    /// Frame whose layout/paint last asked for this image.
    last_used: u64,
}

impl BoundedImageCache {
    pub(crate) fn new(budget_bytes: usize) -> Self {
        Self {
            lru: LruBudget::new(budget_bytes),
            entries: HashMap::new(),
            frame: 0,
        }
    }

    /// Number of resident images (loading requests included).
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Decoded bytes currently counted against the budget.
    pub(crate) fn used_bytes(&self) -> usize {
        self.lru.used
    }

    pub(crate) fn budget_bytes(&self) -> usize {
        self.lru.budget
    }

    pub(crate) fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.len(),
            bytes: self.lru.used,
        }
    }

    /// Loads a resource and accounts for its decoded size once it resolves.
    ///
    /// No eviction happens here: this runs during layout/prepaint/paint, where
    /// dropping an image another element already painted this frame would blank
    /// it (or orphan its atlas texture). Release happens at frame start in
    /// [`BoundedImageCache::begin_frame`].
    pub(crate) fn load(
        &mut self,
        resource: &Resource,
        window: &Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        // Settle decodes that finished for an element that no longer asks for
        // them (fast page navigation, closed overlays). Without this pass such
        // an orphaned result would stay `Loading` forever, hold its decoded
        // bytes and never qualify for eviction.
        self.settle_loading();
        let key = hash(resource);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.frame;
            let loaded = entry.item.get();
            if let Some(Ok(image)) = loaded.as_ref() {
                _ = self.lru.record(key, image_bytes(image));
            }
            self.lru.touch(key);
            loaded
        } else {
            let future = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
            let task = cx.background_executor().spawn(future).shared();
            self.entries.insert(
                key,
                Entry {
                    item: ImageCacheItem::Loading(task.clone()),
                    last_used: self.frame,
                },
            );
            self.lru.touch(key);
            // Wake the requesting view once the decode completes; without this
            // the frame that first asked would stay blank until some other
            // event repainted it.
            let entity = window.current_view();
            window
                .spawn(cx, async move |cx| {
                    _ = task.await;
                    cx.on_next_frame(move |_, cx| {
                        cx.notify(entity);
                    });
                })
                .detach();
            None
        }
    }

    /// Polls every in-flight decode once and records the decoded size of the
    /// ones that finished. `ImageCacheItem::get` is a cheap `now_or_never` on a
    /// still-loading task.
    fn settle_loading(&mut self) {
        let loading: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.item, ImageCacheItem::Loading(_)))
            .map(|(key, _)| *key)
            .collect();
        for key in loading {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            let loaded = entry.item.get();
            let bytes = match loaded.as_ref() {
                Some(Ok(image)) => image_bytes(image),
                Some(Err(_)) | None => 0,
            };
            if loaded.is_some() {
                _ = self.lru.record(key, bytes);
            }
        }
    }

    /// Starts a render frame and unloads images nothing painted for a while.
    ///
    /// A page transition replaces the whole body in one frame: the retired
    /// page's hero and row covers are not requested again, so the next frames
    /// release them here instead of keeping them until the budget forces them
    /// out.
    pub(crate) fn begin_frame(&mut self, window: &mut Window, cx: &mut App) {
        self.frame = self.frame.wrapping_add(1);
        self.evict_stale(window, cx);
        self.evict_budget(window, cx);
    }

    /// Drops images that have not been requested for [`STALE_FRAMES`] frames.
    ///
    /// This runs before layout, so an image painted later in the same frame is
    /// still `STALE_FRAMES` behind and survives; one requested this frame is
    /// never a candidate.
    fn evict_stale(&mut self, window: &mut Window, cx: &mut App) {
        let frame = self.frame;
        let stale: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, entry)| is_stale(frame, entry.last_used))
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.unload(key, window, cx);
        }
    }

    /// Drops least-recently-used loaded images until the budget holds.
    ///
    /// This runs at frame start, before anything is laid out or painted this
    /// frame, so it can never yank an image out from under a paint already in
    /// progress. Loading entries are skipped: dropping their task would discard
    /// an in-flight decode that the requesting frame still needs, and it costs
    /// no decoded bytes yet.
    fn evict_budget(&mut self, window: &mut Window, cx: &mut App) {
        while self.lru.over_budget() {
            let frame = self.frame;
            let Some(key) = self.lru.eviction_candidate(|key| {
                matches!(
                    self.entries.get(&key),
                    Some(Entry {
                        item: ImageCacheItem::Loaded(_),
                        last_used,
                    }) if *last_used < frame
                )
            }) else {
                break;
            };
            self.unload(key, window, cx);
        }
    }

    /// Removes one entry, its byte accounting and (when decoded) its atlas
    /// texture. A still-loading entry is simply forgotten: the task keeps
    /// running to completion but its result is dropped with the handle.
    fn unload(&mut self, key: u64, window: &mut Window, cx: &mut App) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        _ = self.lru.remove(key);
        if let ImageCacheItem::Loaded(Ok(image)) = entry.item {
            cx.drop_image(image, Some(window));
        }
    }
}

impl ImageCache for BoundedImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        BoundedImageCache::load(self, resource, window, cx)
    }
}

/// Whether an image last requested on `last_used` may be unloaded at `frame`.
///
/// Frame numbers only advance when the UI actually renders, so an idle window
/// does not age covers: a paused app that renders no frames keeps the image it
/// showed, and the first frame after a key press still sees `diff == 1`.
fn is_stale(frame: u64, last_used: u64) -> bool {
    frame.saturating_sub(last_used) >= STALE_FRAMES
}

/// Decoded byte length of an image, across all its frames.
fn image_bytes(image: &Arc<RenderImage>) -> usize {
    (0..image.frame_count()).fold(0, |total, frame| {
        total.saturating_add(image.as_bytes(frame).map_or(0, <[u8]>::len))
    })
}

/// LRU order plus decoded-byte accounting, split out from GPUI so the eviction
/// policy is testable without a window or a real image.
#[derive(Debug)]
struct LruBudget {
    budget: usize,
    used: usize,
    bytes: HashMap<u64, usize>,
    /// Front is least recently used; `touch` moves a key to the back.
    order: VecDeque<u64>,
}

impl LruBudget {
    fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            bytes: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Marks `key` as most recently used, inserting it when new.
    fn touch(&mut self, key: u64) {
        if let Some(position) = self.order.iter().position(|existing| *existing == key) {
            self.order.remove(position);
        }
        self.order.push_back(key);
    }

    /// Records the decoded size of `key`; returns `true` when the used total
    /// changed. A `Loading` or failed entry records zero bytes.
    fn record(&mut self, key: u64, bytes: usize) -> bool {
        let previous = self.bytes.insert(key, bytes).unwrap_or(0);
        let before = self.used;
        self.used = self.used.saturating_sub(previous).saturating_add(bytes);
        self.used != before
    }

    /// Forgets `key`, returning the bytes it accounted for.
    fn remove(&mut self, key: u64) -> usize {
        let removed = self.bytes.remove(&key).unwrap_or(0);
        self.used = self.used.saturating_sub(removed);
        if let Some(position) = self.order.iter().position(|existing| *existing == key) {
            self.order.remove(position);
        }
        removed
    }

    fn over_budget(&self) -> bool {
        self.used > self.budget
    }

    /// Oldest key accepted by `is_loaded`; loading entries are skipped.
    fn eviction_candidate(&self, is_loaded: impl Fn(u64) -> bool) -> Option<u64> {
        self.order
            .iter()
            .copied()
            .find(|key| is_loaded(*key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_until_the_budget_holds() {
        let mut lru = LruBudget::new(8);
        for (key, bytes) in [(1, 3), (2, 3), (3, 3)] {
            lru.touch(key);
            assert!(lru.record(key, bytes));
        }
        assert!(lru.over_budget(), "9 decoded bytes exceed the 8 byte budget");
        assert_eq!(lru.eviction_candidate(|_| true), Some(1));
        lru.remove(1);
        assert!(!lru.over_budget());
        assert_eq!(lru.used, 6);
    }

    #[test]
    fn touching_protects_a_key_from_eviction() {
        let mut lru = LruBudget::new(8);
        for key in 1..=3 {
            lru.touch(key);
            _ = lru.record(key, 3);
        }
        lru.touch(1);
        assert_eq!(
            lru.eviction_candidate(|_| true),
            Some(2),
            "1 is now newer than 2"
        );
        lru.remove(2);
        assert_eq!(lru.eviction_candidate(|_| true), Some(3));
    }

    #[test]
    fn failed_entries_cost_no_bytes_and_evict_first() {
        let mut lru = LruBudget::new(4);
        lru.touch(1);
        assert!(!lru.record(1, 0), "a miss records no bytes");
        lru.touch(2);
        _ = lru.record(2, 4);
        assert!(!lru.over_budget());
        assert_eq!(lru.eviction_candidate(|_| true), Some(1));
    }

    #[test]
    fn removing_a_key_returns_its_bytes_and_forgets_the_order_entry() {
        let mut lru = LruBudget::new(16);
        lru.touch(7);
        _ = lru.record(7, 5);
        assert_eq!(lru.remove(7), 5);
        assert_eq!(lru.remove(7), 0, "removal is idempotent");
        assert_eq!(lru.used, 0);
        assert_eq!(lru.eviction_candidate(|_| true), None);
    }

    #[test]
    fn budget_override_is_clamped() {
        assert_eq!(budget_mib_from(None), DEFAULT_BUDGET_MIB);
        assert_eq!(budget_mib_from(Some("0")), MIN_BUDGET_MIB);
        assert_eq!(budget_mib_from(Some(" 32 ")), 32);
        assert_eq!(budget_mib_from(Some("99999")), MAX_BUDGET_MIB);
        assert_eq!(budget_mib_from(Some("junk")), DEFAULT_BUDGET_MIB);
    }

    #[test]
    fn recording_the_same_size_twice_does_not_double_count() {
        let mut lru = LruBudget::new(16);
        lru.touch(4);
        assert!(lru.record(4, 3));
        assert!(!lru.record(4, 3), "an unchanged size changes nothing");
        assert_eq!(lru.used, 3);
        assert!(lru.record(4, 5));
        assert_eq!(lru.used, 5, "a resize replaces the old total");
    }

    #[test]
    fn an_image_is_unloaded_only_after_two_rendered_frames() {
        assert!(!is_stale(10, 10), "requested in this frame");
        assert!(!is_stale(11, 10), "one frame of grace covers a transition");
        assert!(is_stale(12, 10), "left page is released on the second frame");
        assert!(!is_stale(9, 10), "a future stamp (counter wrap) is not stale");
        assert!(!is_stale(0, 0));
    }

    #[test]
    fn empty_cache_reports_zero_usage() {
        let cache = BoundedImageCache::new(8 * BYTES_PER_MIB);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.used_bytes(), 0);
        assert_eq!(cache.stats(), CacheStats::default());
        assert_eq!(cache.budget_bytes(), 8 * BYTES_PER_MIB);
    }
}
