//! Bounded LRU cache for decoded cover art.
//!
//! GPUI's [`gpui::RetainAllImageCache`] never evicts: every decoded image stays
//! in memory (and keeps its sprite-atlas texture) for the process lifetime.
//! The `img()` element without an explicit cache is worse still — its decoded
//! image is held by the app asset cache forever. Cover art is the one payload
//! that grows with every album/artist page a user visits, so entering pages
//! used to increase RSS by the decoded size of each new hero/row cover.
//!
//! [`BoundedImageCache`] implements [`gpui::ImageCache`] with three release
//! rules. An image that no element has requested for a couple of frames is
//! unloaded: leaving a page (or closing an overlay) drops its covers instead
//! of keeping them resident until some later page happens to need the memory.
//! A decoded-byte budget (default 96 MiB) backstops a single screen that
//! really does paint more than that at once. Finally, a small number of the
//! most recently painted **large** covers (the page hero, fullscreen art, the
//! playing cover) stay warm across page changes (`MUSIKA_IMAGE_CACHE_WARM`,
//! default 3): bouncing album -> artist -> album must not re-decode the same
//! image. Everything else still follows the stale-frame rule, and eviction
//! drops the image from the window sprite atlas with
//! [`gpui::App::drop_image`], so neither the CPU pixels nor the texture
//! survive. Only decoded bytes are counted (not the file size): that is the
//! number that matters for RSS.
//!
//! The cache stores no path or URL knowledge; callers hand it a `Resource`
//! through the same `img` element path every surface already uses. The one
//! app-wide cache entity is installed as the root render's image cache, so
//! every cover `img()` in the tree inherits it. Local cover files do not go
//! through that path at all: [`super::fast_image`] decodes them for the
//! painted size and hands back a ready [`RenderImage`].

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::FutureExt as _;
use gpui::{
    App, AppContext, Asset, AssetLogger, Entity, ImageAssetLoader, ImageCache, ImageCacheError,
    RenderImage, Resource, Window, hash,
};

use super::fast_image::{self, quantize_edge};

/// Decoded RGBA budget for cover art. Covers are now decoded at their painted
/// size, so this is a backstop rather than the per-page peak: 96 MiB comfortably
/// holds every visible cover (and a transiently re-decoded larger hero) while
/// still refusing unbounded history.
pub(crate) const DEFAULT_BUDGET_MIB: usize = 96;
const MIN_BUDGET_MIB: usize = 8;
const MAX_BUDGET_MIB: usize = 512;
const BYTES_PER_MIB: usize = 1024 * 1024;

/// Rendered frames an image may go unpainted before it is unloaded. Visible
/// covers are requested on every rendered frame (layout and paint), so two
/// frames of grace covers a page transition without re-decoding the screen it
/// just moved to; frame counting (not wall clock) means an idle window keeps
/// the cover it is showing.
const STALE_FRAMES: u64 = 2;

/// Decoded covers kept resident even when nothing currently paints them.
///
/// The open page's row thumbnails are cheap to reload, but a hero is a real
/// decode. Bouncing between an album and its artist (album -> artist -> album
/// -> ...) or reopening the fullscreen cover otherwise re-decodes the same
/// image every time. The last few large covers stay warm instead; everything
/// else is still released two frames after it stops being painted.
pub(crate) const DEFAULT_WARM_SLOTS: usize = 3;
const MAX_WARM_SLOTS: usize = 8;
/// Smallest decode edge worth keeping warm. Row/player thumbnails (64-384 px)
/// are cheap to re-decode and would otherwise crowd the hero slots.
const WARM_MIN_EDGE: u32 = 512;

/// Absolute longest edge decoded for a local cover file, independent of the
/// display size. This only guards pathological sources and huge transient
/// decodes; painted covers normally decode at their physical screen size.
const DECODE_MAX_EDGE: u32 = 4096;
const DECODE_MIN_EDGE: u32 = 512;
const DECODE_MAX_EDGE_LIMIT: u32 = 8192;

/// Smallest long edge decoded for a painted cover. Row covers are ~40 logical
/// pixels, so this is already generous; the decoded texture is sized for the
/// screen, not for the source file.
const DISPLAY_DECODE_MIN_EDGE: u32 = 64;

/// Long edge (physical pixels) needed to paint a square cover at `logical`
/// points on a `scale_factor` display. One texture pixel per screen pixel is
/// enough for a static image; no supersampling is needed.
pub(crate) fn display_edge_for(logical: f32, scale_factor: f32) -> u32 {
    let edge = (logical.max(0.0) * scale_factor.max(1.0)).ceil();
    edge.clamp(DISPLAY_DECODE_MIN_EDGE as f32, DECODE_MAX_EDGE as f32) as u32
}

/// Creates the single app-wide cover cache.
///
/// `MUSIKA_IMAGE_CACHE_MB` overrides the budget (diagnostics and the memory
/// harness use it); the override is clamped so a typo cannot disable retention
/// or allocate unboundedly.
pub(crate) fn new_cache(cx: &mut App) -> Entity<BoundedImageCache> {
    let budget_mib = budget_mib();
    cx.new(|_| BoundedImageCache::new(budget_mib * BYTES_PER_MIB))
}

/// Parses the `MUSIKA_IMAGE_CACHE_WARM` override, clamped so a typo cannot
/// pin an unbounded number of covers or disable hero retention.
fn warm_slots_from(raw: Option<&str>) -> usize {
    raw.and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_WARM_SLOTS)
        .min(MAX_WARM_SLOTS)
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
    /// Loads served by an existing entry (the decode was reused).
    pub hits: u64,
    /// Loads that had to start a decode.
    pub misses: u64,
}

impl CacheStats {
    pub(crate) fn mib(self) -> f32 {
        self.bytes as f32 / BYTES_PER_MIB as f32
    }
}

/// Whether an entry may occupy a warm slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retain {
    Never,
    IfLarge,
    Always,
}

/// An [`ImageCache`] that unloads images nothing paints anymore.
pub(crate) struct BoundedImageCache {
    lru: LruBudget,
    entries: HashMap<u64, Entry>,
    /// Decode cache hit/miss counters: one miss per distinct cover resource,
    /// every other surface painting the same cover is a hit.
    hits: u64,
    misses: u64,
    /// Monotonic render-frame counter. A cache entity sees one `begin_frame`
    /// per rendered frame; loads during that frame mark their image as painted.
    frame: u64,
    /// Monotonic paint counter: recency for the warm slots, with a finer
    /// ordering than `frame` when several surfaces are painted in one frame.
    touch_seq: u64,
    /// Large decoded images retained across page changes (see
    /// [`DEFAULT_WARM_SLOTS`]).
    warm_slots: usize,
}

/// One decode in flight. The task is owned by the cache entry, so dropping
/// the entry cancels the decode: a page navigation, an edge growth or a
/// budget eviction cannot leave a superseded 4000 px decode burning a
/// background thread for a cover nothing paints anymore.
type DecodeFuture =
    Pin<Box<dyn Future<Output = Result<Arc<RenderImage>, ImageCacheError>> + Send + 'static>>;

/// State of one cached cover.
enum EntryItem {
    Loading(DecodeFuture),
    Loaded(Result<Arc<RenderImage>, ImageCacheError>),
}

struct Entry {
    item: EntryItem,
    /// Frame whose layout/paint last asked for this image.
    last_used: u64,
    /// Paint-order recency, used to rank warm candidates.
    last_touch: u64,
    /// Long edge this local image was decoded for (0 for remote/stock images).
    /// A later surface needing a bigger texture re-decodes once.
    edge: u32,
    /// Whether this entry may occupy a warm slot across page changes.
    retain: bool,
}

impl BoundedImageCache {
    pub(crate) fn new(budget_bytes: usize) -> Self {
        Self {
            lru: LruBudget::new(budget_bytes),
            entries: HashMap::new(),
            hits: 0,
            misses: 0,
            frame: 0,
            touch_seq: 0,
            warm_slots: warm_slots_from(std::env::var("MUSIKA_IMAGE_CACHE_WARM").ok().as_deref()),
        }
    }

    /// Number of large covers retained across page changes.
    pub(crate) fn warm_slots(&self) -> usize {
        self.warm_slots
    }

    /// Overrides the warm-slot count; tests and diagnostics only.
    #[cfg(test)]
    pub(crate) fn set_warm_slots(&mut self, slots: usize) {
        self.warm_slots = slots.min(MAX_WARM_SLOTS);
    }

    /// Marks one entry as most recently painted.
    fn touch(&mut self, key: u64) {
        self.touch_seq = self.touch_seq.wrapping_add(1);
        let touch_seq = self.touch_seq;
        let frame = self.frame;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = frame;
            entry.last_touch = touch_seq;
        }
        self.lru.touch(key);
    }

    /// Keys of the large decoded images protected from stale eviction.
    fn warm_keys(&self) -> HashSet<u64> {
        if self.warm_slots == 0 {
            return HashSet::new();
        }
        let mut candidates: Vec<(u64, u64)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.retain && matches!(entry.item, EntryItem::Loaded(_)))
            .map(|(key, entry)| (*key, entry.last_touch))
            .collect();
        candidates.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
        candidates
            .into_iter()
            .take(self.warm_slots)
            .map(|(key, _)| key)
            .collect()
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
            hits: self.hits,
            misses: self.misses,
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
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.load_with_edge(resource, None, Retain::Never, window, cx)
    }

    /// Loads a cover painted at `display_edge` physical pixels. Local files
    /// smaller than that decode no further; local files decoded earlier for a
    /// smaller surface are re-decoded once when a larger surface needs them.
    pub(crate) fn load_display(
        &mut self,
        resource: &Resource,
        display_edge: u32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.load_with_edge(resource, Some(display_edge), Retain::IfLarge, window, cx)
    }

    /// Loads a hero/player-bar cover that stays warm across page changes even
    /// when it is currently not painted (the identity closure must resolve it
    /// again without a decode).
    pub(crate) fn load_display_retained(
        &mut self,
        resource: &Resource,
        display_edge: u32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.load_with_edge(resource, Some(display_edge), Retain::Always, window, cx)
    }

    /// Loads a cheap placeholder thumbnail. Previews never occupy a warm slot:
    /// they are a few KB and re-decoded instantly, while a warm slot must hold
    /// one of the large covers the user is actually navigating between.
    pub(crate) fn load_preview(
        &mut self,
        resource: &Resource,
        display_edge: u32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        self.load_with_edge(resource, Some(display_edge), Retain::Never, window, cx)
    }

    fn load_with_edge(
        &mut self,
        resource: &Resource,
        display_edge: Option<u32>,
        retain: Retain,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        // Settle decodes that finished for an element that no longer asks for
        // them (fast page navigation, closed overlays). Without this pass such
        // an orphaned result would stay `Loading` forever, hold its decoded
        // bytes and never qualify for eviction.
        self.settle_loading(window);
        let key = hash(resource);
        let local = cappable_path(resource).is_some();
        let edge = if local {
            let requested = display_edge.unwrap_or_else(decode_max_edge);
            // Decode edges are quantized so one decoded texture serves a
            // range of window sizes (hero -> fullscreen) instead of forcing a
            // new full decode for every pixel change.
            quantize_edge(
                requested
                    .min(decode_max_edge())
                    .max(DISPLAY_DECODE_MIN_EDGE),
            )
        } else {
            0
        };
        let retain = match retain {
            Retain::Never => false,
            Retain::Always => true,
            // Remote provider art is already a "hero" resource: keep it warm
            // when a surface asked for it by display size.
            Retain::IfLarge => !local || edge >= WARM_MIN_EDGE,
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            // A same-size or larger decode already serves this request. A
            // smaller one must grow once (the page hero at 400 px, then the
            // fullscreen cover at 900 px).
            if !(local && edge > entry.edge) {
                self.hits = self.hits.saturating_add(1);
                entry.retain |= retain;
                let loaded = match &entry.item {
                    EntryItem::Loading(_) => None,
                    EntryItem::Loaded(result) => Some(result.clone()),
                };
                if let Some(Ok(image)) = loaded.as_ref() {
                    _ = self.lru.record(key, image_bytes(image));
                }
                self.touch(key);
                return loaded;
            }
            self.unload(key, window, cx);
        }
        self.misses = self.misses.saturating_add(1);
        let task = load_image_task(resource, cx, edge);
        self.entries.insert(
            key,
            Entry {
                item: EntryItem::Loading(task),
                last_used: self.frame,
                last_touch: 0,
                edge,
                retain,
            },
        );
        self.touch(key);
        // Keep frames coming until the decode settles: `settle_loading` runs
        // at the next frame start and swaps the finished task for its image.
        window.request_animation_frame();
        None
    }

    /// Polls every in-flight decode once and records the decoded size of the
    /// ones that finished. Decodes run on the background executor; polling at
    /// frame start means no per-task wakeup closure is needed, and dropping a
    /// pending entry (navigation, eviction) cancels its decode instead of
    /// letting it finish unobserved.
    fn settle_loading(&mut self, window: &Window) {
        let loading: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry.item, EntryItem::Loading(_)))
            .map(|(key, _)| *key)
            .collect();
        let mut pending = false;
        for key in loading {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            let mut finished = None;
            if let EntryItem::Loading(future) = &mut entry.item {
                finished = (&mut *future).now_or_never();
            }
            let Some(result) = finished else {
                pending = true;
                continue;
            };
            let bytes = result.as_ref().map_or(0, image_bytes);
            _ = self.lru.record(key, bytes);
            entry.item = EntryItem::Loaded(result);
        }
        if pending {
            window.request_animation_frame();
        }
    }

    /// Releases the retired page's covers immediately, keeping the warm slots.
    ///
    /// Called when the open page identity changes. Row thumbnails, collage
    /// panes and previews are cheap to reload, so they are dropped at once
    /// instead of waiting for the stale-frame grace; the last few large covers
    /// (page heroes, fullscreen art, the playing cover) stay resident so
    /// album -> artist -> album does not re-decode them.
    pub(crate) fn purge_retired(&mut self, window: &mut Window, cx: &mut App) {
        let warm = self.warm_keys();
        let keys: Vec<u64> = self
            .entries
            .keys()
            .copied()
            .filter(|key| !warm.contains(key))
            .collect();
        for key in keys {
            self.unload(key, window, cx);
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
        self.settle_loading(window);
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
        let warm = self.warm_keys();
        let stale: Vec<u64> = self
            .entries
            .iter()
            .filter(|(key, entry)| is_stale(frame, entry.last_used) && !warm.contains(key))
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
            let warm = self.warm_keys();
            let evictable = |key: u64| {
                matches!(
                    self.entries.get(&key),
                    Some(Entry {
                        item: EntryItem::Loaded(_),
                        last_used,
                        ..
                    }) if *last_used < frame
                )
            };
            // Prefer dropping non-warm images; if only warm ones are left, the
            // decoded-byte budget is a hard cap and they go too.
            let key = self
                .lru
                .eviction_candidate(|key| evictable(key) && !warm.contains(&key))
                .or_else(|| self.lru.eviction_candidate(evictable));
            let Some(key) = key else {
                break;
            };
            self.unload(key, window, cx);
        }
    }

    /// Removes one entry, its byte accounting and (when decoded) its atlas
    /// texture. A still-loading entry is cancelled: dropping the owned task
    /// aborts the background decode instead of letting it finish unobserved.
    fn unload(&mut self, key: u64, window: &mut Window, cx: &mut App) {
        let Some(entry) = self.entries.remove(&key) else {
            return;
        };
        _ = self.lru.remove(key);
        if let EntryItem::Loaded(Ok(image)) = entry.item {
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

/// Builds the loading task for one image: local files decode through the
/// display-sized fast decoder; everything else (remote provider art, embedded
/// assets) keeps gpui's stock loader.
fn load_image_task(resource: &Resource, cx: &mut App, max_edge: u32) -> DecodeFuture {
    let Some(path) = cappable_path(resource) else {
        let future = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
        return Box::pin(cx.background_executor().spawn(future));
    };
    let max_edge = if max_edge == 0 {
        decode_max_edge()
    } else {
        max_edge
    };
    Box::pin(
        cx.background_executor()
            .spawn(async move { fast_image::decode_for_display(&path, max_edge) }),
    )
}

/// Local files that decode through [`decode_capped_file`]. Animated cover
/// formats (GIF) and SVG-backed assets keep gpui's loader.
fn cappable_path(resource: &Resource) -> Option<Arc<std::path::Path>> {
    let Resource::Path(path) = resource else {
        return None;
    };
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff"
    )
    .then(|| Arc::clone(path))
}

fn decode_max_edge() -> u32 {
    decode_edge_from(std::env::var("MUSIKA_COVER_MAX_EDGE").ok().as_deref())
}

/// Parses the decode-cap override, defaulting and clamping so a typo can
/// neither reintroduce the pathological single-image spike nor blur covers.
fn decode_edge_from(raw: Option<&str>) -> u32 {
    raw.and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(DECODE_MAX_EDGE)
        .clamp(DECODE_MIN_EDGE, DECODE_MAX_EDGE_LIMIT)
}

/// Decodes one local cover for a surface painted at `max_edge` physical px.
///
/// Kept as the cache/benchmark entry point; [`fast_image::decode_for_display`]
/// owns the DCT-scaled JPEG path, SIMD resize and display-derivative cache.
pub(crate) fn decode_capped_file(
    path: &std::path::Path,
    max_edge: u32,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    fast_image::decode_for_display(path, max_edge)
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
    fn display_edge_tracks_screen_pixels() {
        assert_eq!(display_edge_for(40.0, 1.0), 64);
        assert_eq!(display_edge_for(400.0, 1.0), 400);
        assert_eq!(display_edge_for(400.0, 2.0), 800);
        assert_eq!(display_edge_for(10_000.0, 2.0), DECODE_MAX_EDGE);
    }

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
    fn decode_cap_override_is_clamped() {
        assert_eq!(decode_edge_from(None), DECODE_MAX_EDGE);
        assert_eq!(decode_edge_from(Some("100")), DECODE_MIN_EDGE);
        assert_eq!(decode_edge_from(Some(" 2048 ")), 2048);
        assert_eq!(decode_edge_from(Some("99999")), DECODE_MAX_EDGE_LIMIT);
        assert_eq!(decode_edge_from(Some("junk")), DECODE_MAX_EDGE);
    }

    fn temp_image(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "musika-decode-{}-{stamp}-{name}",
            std::process::id()
        ))
    }

    fn write_jpeg(width: u32, height: u32, pixel: [u8; 3]) -> std::path::PathBuf {
        let path = temp_image("cover.jpg");
        let image = image::RgbImage::from_pixel(width, height, image::Rgb(pixel));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .expect("encode fixture");
        std::fs::write(&path, bytes.into_inner()).expect("write fixture");
        path
    }

    #[test]
    fn oversized_photos_decode_downscaled_and_small_ones_untouched() {
        let large = write_jpeg(5000, 2500, [10, 20, 30]);
        let decoded = decode_capped_file(&large, 4096).expect("decode large fixture");
        let size = decoded.size(0);
        assert_eq!((size.width.0, size.height.0), (4096, 2048));
        // gpui expects BGRA; the RGB fixture becomes B,G,R,A after the swap.
        let first = &decoded.as_bytes(0).expect("frame bytes")[..4];
        assert_eq!(first, [30, 20, 10, 255]);
        _ = std::fs::remove_file(&large);

        let small = write_jpeg(320, 200, [1, 2, 3]);
        let decoded = decode_capped_file(&small, 4096).expect("decode small fixture");
        let size = decoded.size(0);
        assert_eq!((size.width.0, size.height.0), (320, 200));
        _ = std::fs::remove_file(&small);
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
        assert!(
            is_stale(12, 10),
            "left page is released on the second frame"
        );
        assert!(
            !is_stale(9, 10),
            "a future stamp (counter wrap) is not stale"
        );
        assert!(!is_stale(0, 0));
    }

    #[test]
    fn warm_slot_count_parses_and_clamps() {
        assert_eq!(warm_slots_from(None), DEFAULT_WARM_SLOTS);
        assert_eq!(warm_slots_from(Some("0")), 0);
        assert_eq!(warm_slots_from(Some(" 2 ")), 2);
        assert_eq!(warm_slots_from(Some("99")), MAX_WARM_SLOTS);
        assert_eq!(warm_slots_from(Some("junk")), DEFAULT_WARM_SLOTS);
    }

    fn insert_loaded(cache: &mut BoundedImageCache, key: u64, retain: bool, last_touch: u64) {
        cache.entries.insert(
            key,
            Entry {
                item: EntryItem::Loaded(Err(ImageCacheError::Io(Arc::new(std::io::Error::other(
                    "test image",
                ))))),
                last_used: 0,
                last_touch,
                edge: 512,
                retain,
            },
        );
    }

    #[test]
    fn warm_keys_keep_the_most_recently_painted_large_covers() {
        let mut cache = BoundedImageCache::new(8 * BYTES_PER_MIB);
        cache.set_warm_slots(2);
        insert_loaded(&mut cache, 1, true, 10);
        insert_loaded(&mut cache, 2, true, 30);
        insert_loaded(&mut cache, 3, false, 50);
        insert_loaded(&mut cache, 4, true, 20);
        insert_loaded(&mut cache, 5, false, 40);
        assert_eq!(cache.warm_keys(), HashSet::from([2, 4]));
    }

    #[test]
    fn zero_warm_slots_disable_retention() {
        let mut cache = BoundedImageCache::new(8 * BYTES_PER_MIB);
        cache.set_warm_slots(0);
        insert_loaded(&mut cache, 1, true, 10);
        assert!(cache.warm_keys().is_empty());
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
