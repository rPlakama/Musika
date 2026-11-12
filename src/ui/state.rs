// Musika — GPUUI shell/list state (M1 first slice).

use std::sync::Arc;

use super::scroll_modal;
use crate::ui::widgets::media::{CoverSource, cover_sources};

pub use crate::model::{PageDescriptor, PageHeader, PageKind, Row, RowKind, Scope};

pub use crate::model::SEARCH_VISIBLE_ROWS;

/// The legacy UI `RowWindow` contract: a count plus only the rows inside the live
/// window. The UI never materializes the whole library.
#[derive(Debug, Clone, Default)]
pub struct RowWindow {
    pub generation: u64,
    pub count: usize,
    pub live_start: usize,
    pub live: Vec<Row>,
    /// The absolute range of the latest request, used to dedupe band changes.
    pub requested: Option<(usize, usize)>,
}

impl RowWindow {
    pub fn full(rows: Vec<Row>) -> Self {
        Self {
            generation: 0,
            count: rows.len(),
            live_start: 0,
            live: rows,
            requested: None,
        }
    }

    pub fn clear(&mut self) {
        self.generation = 0;
        self.count = 0;
        self.live_start = 0;
        self.live.clear();
        self.requested = None;
    }

    pub fn install_count(&mut self, generation: u64, count: usize) {
        if generation != self.generation {
            self.live.clear();
            self.live_start = 0;
            self.requested = None;
        }
        self.generation = generation;
        self.count = count;
    }

    pub fn install_window(&mut self, start: usize, rows: Vec<Row>) {
        self.live_start = start;
        self.live = rows;
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        if index < self.live_start {
            return None;
        }
        self.live.get(index - self.live_start)
    }

    pub fn covers(&self, start: usize, end: usize) -> bool {
        match self.requested {
            Some((lo, hi)) => lo <= start && end <= hi,
            None => false,
        }
    }

    pub fn set_requested(&mut self, start: usize, end: usize) {
        self.requested = Some((start, end));
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, index: usize) -> Option<&Row> {
        self.row(index)
    }
}

/// The state every list surface shares: the cursor position and the visual
/// range anchor. Each surface owns its rows (a [`RowWindow`], a page's row
/// vector, the live queue) and reads the cursor through this state.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListState {
    pub cursor: usize,
    pub selection: Selection,
}

impl ListState {
    /// Clamp `index` into a list of `len` rows (an empty list keeps the head).
    pub fn select(&mut self, index: usize, len: usize) {
        self.cursor = if len == 0 { 0 } else { index.min(len - 1) };
    }

    /// The id of the row under the cursor, or `None` on an empty surface.
    pub fn row_id<'a>(&self, rows: impl FnOnce(usize) -> Option<&'a Row>) -> Option<&'a str> {
        rows(self.cursor).map(|row| row.id.as_str())
    }

    /// Back to the head with no visual range.
    pub fn clear(&mut self) {
        self.cursor = 0;
        self.selection.clear();
    }
}

/// Start the scroll-follow animation for one list surface: the surface's
/// [`ListState`] owns the cursor, its `ScrollModal` owns the viewport metrics.
pub fn follow_list(
    list: &ListState,
    scroll: &mut scroll_modal::ScrollModal,
    count: usize,
    animated: bool,
    now: f64,
) {
    scroll.start(count, list.cursor, animated, now);
}

/// Land the scroll modal on `index`, clamp the shared cursor to the list and
/// return the landed row. One implementation for every list surface.
pub fn land_list(
    list: &mut ListState,
    scroll: &mut scroll_modal::ScrollModal,
    count: usize,
    index: usize,
) -> usize {
    let cursor = scroll.land(count, index);
    list.select(cursor, count);
    cursor
}

#[derive(Debug, Clone)]
pub struct ViewState {
    pub scope: Scope,
    pub order: usize,
    pub rows: RowWindow,
    pub list: ListState,
    pub loading: bool,
    pub error: Option<String>,
    /// Container state for the superior bar's contextual chips (Album/Artist).
    pub container_cached: bool,
    pub container_downloading: bool,
    /// Resolved cover per live row (offset by `rows.live_start`), built when a
    /// window/payload is installed instead of per frame.
    pub covers: Vec<Option<Arc<CoverSource>>>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scope: Scope::Albums,
            order: 0,
            rows: RowWindow::default(),
            list: ListState::default(),
            loading: true,
            error: None,
            container_cached: false,
            container_downloading: false,
            covers: Vec::new(),
        }
    }
}

impl ViewState {
    pub fn order_label(&self) -> &'static str {
        match self.scope {
            Scope::Albums | Scope::Songs => {
                crate::library::ORDERS[self.order.min(crate::library::ORDERS.len() - 1)]
            }
            Scope::Artists => {
                crate::library::ARTIST_ORDERS
                    [self.order.min(crate::library::ARTIST_ORDERS.len() - 1)]
            }
            Scope::Playlists => "Name",
        }
    }

    pub fn order_len(&self) -> usize {
        match self.scope {
            Scope::Albums | Scope::Songs => crate::library::ORDERS.len(),
            Scope::Artists => crate::library::ARTIST_ORDERS.len(),
            Scope::Playlists => 1,
        }
    }

    pub fn clear_for(&mut self, scope: Scope) {
        self.list.clear();
        self.scope = scope;
        self.order = 0;
        self.rows.clear();
        self.covers.clear();
        self.loading = true;
        self.error = None;
    }

    /// Resolve the live row covers once, when the row window lands.
    pub fn rebuild_covers(&mut self) {
        self.covers = cover_sources(
            self.rows
                .live
                .iter()
                .map(|row| (row.cover_url.as_str(), row.cover_remote.as_str())),
        );
    }

    /// Cover for an absolute row index; `None` outside the live window.
    pub fn cover(&self, index: usize) -> Option<&Arc<CoverSource>> {
        let offset = index.checked_sub(self.rows.live_start)?;
        self.covers.get(offset).and_then(|cover| cover.as_ref())
    }

    pub fn select(&mut self, index: usize) {
        self.list.select(index, self.rows.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_cycle_in_both_directions() {
        let mut scope = Scope::Albums;
        for _ in 0..4 {
            scope = scope.next();
        }
        assert_eq!(scope, Scope::Albums);
        for _ in 0..4 {
            scope = scope.prev();
        }
        assert_eq!(scope, Scope::Albums);
    }

    #[test]
    fn cursor_is_clamped_to_rows() {
        let mut view = ViewState {
            rows: RowWindow::full(vec![Row {
                track: String::new(),
                id: "a".into(),
                kind: RowKind::Album,
                title: "A".into(),
                subtitle: String::new(),
                meta: String::new(),
                cover_url: String::new(),
                cover_remote: String::new(),
                song_id: String::new(),
                album_id: "a".into(),
                starred: false,
                rating: 0,
                downloaded: false,
            }]),
            ..ViewState::default()
        };
        view.select(99);
        assert_eq!(view.list.cursor, 0);
        view.rows.clear();
        view.select(4);
        assert_eq!(view.list.cursor, 0);
    }

    #[test]
    fn order_labels_follow_the_scope() {
        let mut view = ViewState::default();
        assert_eq!(view.order_label(), "Artist");
        view.scope = Scope::Artists;
        view.order = 1;
        assert_eq!(view.order_label(), "Albums");
        assert_eq!(view.order_len(), 2);
    }

    #[test]
    fn row_window_only_exposes_loaded_rows() {
        let mut window = RowWindow::default();
        window.install_count(7, 50);
        assert_eq!(window.len(), 50);
        assert!(window.row(0).is_none());
        window.install_window(
            10,
            vec![Row {
                track: String::new(),
                id: "ten".into(),
                kind: RowKind::Album,
                title: "Ten".into(),
                subtitle: String::new(),
                meta: String::new(),
                cover_url: String::new(),
                cover_remote: String::new(),
                song_id: String::new(),
                album_id: "ten".into(),
                starred: false,
                rating: 0,
                downloaded: false,
            }],
        );
        assert!(window.row(9).is_none());
        assert_eq!(window.row(10).map(|row| row.id.as_str()), Some("ten"));
        assert!(window.row(11).is_none());
        assert!(!window.covers(0, 10));
        window.set_requested(0, 30);
        assert!(window.covers(0, 10));
        window.install_count(8, 4);
        assert_eq!(window.len(), 4);
        assert!(window.row(10).is_none());
    }
}

#[derive(Debug, Clone)]
pub struct PageState {
    pub descriptor: PageDescriptor,
    pub header: PageHeader,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub loading: bool,
    pub error: Option<String>,
    /// Container state for the superior bar's contextual chips (Album/Artist).
    pub container_cached: bool,
    pub container_downloading: bool,
    /// Resolved cover per row, rebuilt when the page payload lands.
    pub covers: Vec<Option<Arc<CoverSource>>>,
}

impl PageState {
    pub fn new(descriptor: PageDescriptor) -> Self {
        Self {
            descriptor,
            header: PageHeader {
                title: String::new(),
                subtitle: String::new(),
                meta: String::new(),
                cover_url: String::new(),
                cover_remote: String::new(),
                collage: Vec::new(),
                collage_remote: Vec::new(),
                collage_art: Vec::new(),
                cover_art: String::new(),
                rating: 0,
                starred: false,
            },
            rows: Vec::new(),
            list: ListState::default(),
            loading: true,
            error: None,
            container_cached: false,
            container_downloading: false,
            covers: Vec::new(),
        }
    }

    pub fn rebuild_covers(&mut self) {
        self.covers = cover_sources(
            self.rows
                .iter()
                .map(|row| (row.cover_url.as_str(), row.cover_remote.as_str())),
        );
    }

    pub fn cover(&self, index: usize) -> Option<&Arc<CoverSource>> {
        self.covers.get(index).and_then(|cover| cover.as_ref())
    }

    pub fn select(&mut self, index: usize) {
        self.list.select(index, self.rows.len());
    }
}

/// Travel log: the legacy UI shell keeps the last 32 places. M1 keeps descriptors
/// (page content is reloaded on travel instead of cached).
#[derive(Default)]
pub struct Travel {
    pub places: Vec<PageDescriptor>,
    pub position: usize,
}

impl Travel {
    pub fn open(&mut self, descriptor: PageDescriptor) {
        if self.current() == Some(&descriptor) {
            return;
        }
        self.places.truncate(self.position.saturating_add(1));
        self.places.push(descriptor);
        if self.places.len() > 32 {
            self.places.remove(0);
        }
        self.position = self.places.len().saturating_sub(1);
    }

    /// Ends the travel session (arriving at Home starts a fresh one).
    pub fn clear(&mut self) {
        self.places.clear();
        self.position = 0;
    }

    pub fn back(&mut self) -> Option<PageDescriptor> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        self.places.get(self.position).cloned()
    }

    pub fn step(&mut self, delta: i32) -> Option<PageDescriptor> {
        let next = self.position as i32 + delta;
        if next < 0 || next as usize >= self.places.len() {
            return None;
        }
        self.position = next as usize;
        self.places.get(self.position).cloned()
    }

    pub fn current(&self) -> Option<&PageDescriptor> {
        self.places.get(self.position)
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn radio_descriptor_keeps_seed_kind() {
        let descriptor = PageDescriptor::radio("album", "a1");
        assert_eq!(descriptor.kind, PageKind::Radio);
        assert_eq!(descriptor.id, "a1");
        assert_eq!(descriptor.radio_seed_kind.as_deref(), Some("album"));
        assert_eq!(PageKind::Radio.label(), "Radio");
        assert_eq!(
            PageDescriptor::new(PageKind::Album, "x").radio_seed_kind,
            None
        );
    }

    fn test_row() -> Row {
        Row {
            track: String::new(),
            id: "r".into(),
            kind: RowKind::Song,
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
            cover_url: String::new(),
            cover_remote: String::new(),
            song_id: String::new(),
            album_id: String::new(),
            starred: false,
            rating: 0,
            downloaded: false,
        }
    }

    #[test]
    fn view_cover_cache_follows_the_live_window_offset() {
        let mut view = ViewState::default();
        view.rows.install_window(10, vec![test_row(), test_row()]);
        view.rebuild_covers();
        assert_eq!(view.covers.len(), 2);
        assert!(view.cover(9).is_none(), "before the live window");
        assert!(view.cover(12).is_none(), "past the live window");
        assert!(view.cover(10).is_none(), "no cover url resolves to none");
    }

    #[test]
    fn page_cover_cache_is_indexed_by_row() {
        let mut page = PageState::new(PageDescriptor::new(PageKind::Album, "a"));
        page.rows = vec![test_row(), test_row(), test_row()];
        page.rebuild_covers();
        assert_eq!(page.covers.len(), 3);
        assert!(page.cover(3).is_none());
    }

    #[test]
    fn home_clear_prevents_stale_back_targets() {
        let a = PageDescriptor::new(PageKind::Album, "a");
        let b = PageDescriptor::new(PageKind::Album, "b");
        let mut travel = Travel::default();
        travel.open(a.clone());
        // Back from A reaches Home (no entry); the app then clears the log.
        assert!(travel.back().is_none());
        travel.clear();
        travel.open(b);
        // Back from B must reach Home, not the previous session's A.
        assert!(travel.back().is_none());
        assert_eq!(travel.places.len(), 1);
    }

    #[test]
    fn travel_log_back_and_forward() {
        let mut travel = Travel::default();
        travel.open(PageDescriptor::new(PageKind::Album, "a"));
        travel.open(PageDescriptor::new(PageKind::Artist, "b"));
        assert_eq!(travel.back().unwrap().id, "a");
        assert_eq!(travel.step(1).unwrap().id, "b");
        assert!(travel.step(1).is_none());
    }

    #[test]
    fn travel_skips_a_duplicate_consecutive_page() {
        let mut travel = Travel::default();
        let album = PageDescriptor::new(PageKind::Album, "same");
        travel.open(album.clone());
        travel.open(album.clone());
        assert_eq!(travel.places.len(), 1);
        assert!(travel.back().is_none());
    }

    #[test]
    fn travel_truncates_forward_history_on_open() {
        let mut travel = Travel::default();
        for id in ["a", "b", "c"] {
            travel.open(PageDescriptor::new(PageKind::Album, id));
        }
        travel.back();
        travel.open(PageDescriptor::new(PageKind::Album, "d"));
        assert_eq!(travel.places.len(), 3);
        assert_eq!(travel.current().unwrap().id, "d");
    }
}

#[cfg(test)]
mod page_kind_tests {
    use super::*;

    #[test]
    fn page_select_clamps() {
        let mut page = PageState::new(PageDescriptor::new(PageKind::Album, "a"));
        page.select(4);
        assert_eq!(page.list.cursor, 0);
        page.rows = vec![Row {
            track: String::new(),
            id: "s".into(),
            kind: RowKind::Song,
            title: "S".into(),
            subtitle: String::new(),
            meta: String::new(),
            cover_url: String::new(),
            cover_remote: String::new(),
            song_id: "s".into(),
            album_id: "a".into(),
            starred: false,
            rating: 0,
            downloaded: false,
        }];
        page.select(9);
        assert_eq!(page.list.cursor, 0);
    }
}

/// Visual selection over the live virtual window. The cursor is owned by the
/// view; this remembers only the anchor. (G2: range ids outside the window come
/// from a backend query in G2's next slice.)
#[derive(Debug, Clone, Copy, Default)]
pub struct Selection {
    anchor: Option<usize>,
}

impl Selection {
    pub fn active(&self) -> bool {
        self.anchor.is_some()
    }

    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    pub fn start(&mut self, cursor: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(cursor);
        }
    }

    pub fn clear(&mut self) {
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
    }

    pub fn contains(&self, cursor: usize, index: usize) -> bool {
        let Some(anchor) = self.anchor else {
            return false;
        };
        let lo = anchor.min(cursor);
        let hi = anchor.max(cursor);
        index >= lo && index <= hi
    }

    pub fn count(&self, cursor: usize) -> usize {
        self.anchor
            .map(|anchor| anchor.abs_diff(cursor) + 1)
            .unwrap_or(0)
    }

    /// The inclusive indices this selection covers, ascending. `row_only`
    /// and an inactive selection both mean just the cursor row.
    /// Shift an active range anchor (a block move keeps the range selected).
    pub fn shift(&mut self, delta: i32) {
        let Some(anchor) = self.anchor else {
            return;
        };
        let next = anchor as i64 + i64::from(delta);
        if next >= 0 {
            self.anchor = Some(next as usize);
        }
    }

    pub fn indices(&self, cursor: usize, row_only: bool) -> Vec<usize> {
        let Some(anchor) = self.anchor else {
            return vec![cursor];
        };
        if row_only {
            return vec![cursor];
        }
        let lo = anchor.min(cursor);
        let hi = anchor.max(cursor);
        (lo..=hi).collect()
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn selection_indices_are_ascending_and_row_only_short_circuits() {
        let mut selection = Selection::default();
        assert_eq!(selection.indices(7, false), vec![7]);
        selection.start(5);
        assert_eq!(selection.indices(8, false), vec![5, 6, 7, 8]);
        assert_eq!(selection.indices(2, false), vec![2, 3, 4, 5]);
        assert_eq!(selection.indices(8, true), vec![8]);
        selection.clear();
        assert_eq!(selection.indices(3, false), vec![3]);
    }

    #[test]
    fn shifting_keeps_a_block_selection_in_sync() {
        let mut selection = Selection::default();
        selection.start(3);
        selection.shift(-1);
        assert_eq!(selection.anchor(), Some(2));
        selection.shift(1);
        assert_eq!(selection.anchor(), Some(3));
        selection.clear();
        selection.shift(1);
        assert!(!selection.active());
    }

    #[test]
    fn visual_range_tracks_both_cursor_directions() {
        let mut selection = Selection::default();
        selection.start(5);
        assert!(selection.contains(8, 6));
        assert!(selection.contains(8, 8));
        assert!(!selection.contains(8, 9));
        assert!(selection.contains(2, 3));
        assert_eq!(selection.count(8), 4);
        selection.clear();
        assert!(!selection.active());
    }
}
