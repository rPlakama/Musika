// Musika — GPUUI shell/list state (M1 first slice).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Albums,
    Songs,
    Artists,
    Playlists,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Albums, Scope::Songs, Scope::Artists, Scope::Playlists];

    pub fn key(self) -> &'static str {
        match self {
            Scope::Albums => "albums",
            Scope::Songs => "songs",
            Scope::Artists => "artists",
            Scope::Playlists => "playlists",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Albums => "Albums",
            Scope::Songs => "Songs",
            Scope::Artists => "Artists",
            Scope::Playlists => "Playlists",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Scope::Albums => Scope::Songs,
            Scope::Songs => Scope::Artists,
            Scope::Artists => Scope::Playlists,
            Scope::Playlists => Scope::Albums,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Scope::Albums => Scope::Playlists,
            Scope::Songs => Scope::Albums,
            Scope::Artists => Scope::Songs,
            Scope::Playlists => Scope::Artists,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Album,
    Song,
    Artist,
    Playlist,
}

/// Number of search results a scope can paint; also the backend query cap.
pub const SEARCH_VISIBLE_ROWS: usize = 5;
/// Rows requested from the backend; the overlay scrolls beyond the visible 5.
pub const SEARCH_FETCH_ROWS: usize = 100;

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub kind: RowKind,
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    /// Album track number/module text, searchable by `/` but not painted.
    pub track: String,
    pub cover_url: String,
    pub song_id: String,
    pub album_id: String,
    pub starred: bool,
    pub rating: i64,
    pub downloaded: bool,
}

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

#[derive(Debug, Clone)]
pub struct ViewState {
    pub scope: Scope,
    pub order: usize,
    pub rows: RowWindow,
    pub cursor: usize,
    pub scroll: f32,
    pub loading: bool,
    pub error: Option<String>,
    pub selection: Selection,
    /// Container state for the superior bar's contextual chips (Album/Artist).
    pub container_cached: bool,
    pub container_downloading: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scope: Scope::Albums,
            order: 0,
            rows: RowWindow::default(),
            cursor: 0,
            scroll: 0.0,
            loading: true,
            error: None,
            selection: Selection::default(),
            container_cached: false,
            container_downloading: false,
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
                crate::library::ARTIST_ORDERS[self.order.min(crate::library::ARTIST_ORDERS.len() - 1)]
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
        self.selection.clear();
        self.scope = scope;
        self.order = 0;
        self.rows.clear();
        self.cursor = 0;
        self.scroll = 0.0;
        self.loading = true;
        self.error = None;
    }

    pub fn select(&mut self, index: usize) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = index.min(self.rows.len() - 1);
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
                song_id: String::new(),
                album_id: "a".into(),
                starred: false,
                rating: 0,
                downloaded: false,
            }]),
            ..ViewState::default()
        };
        view.select(99);
        assert_eq!(view.cursor, 0);
        view.rows.clear();
        view.select(4);
        assert_eq!(view.cursor, 0);
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
        window.install_window(10, vec![Row {
            track: String::new(),
            id: "ten".into(),
            kind: RowKind::Album,
            title: "Ten".into(),
            subtitle: String::new(),
            meta: String::new(),
            cover_url: String::new(),
            song_id: String::new(),
            album_id: "ten".into(),
            starred: false,
            rating: 0,
            downloaded: false,
        }]);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageKind {
    Album,
    Artist,
    Playlist,
    Radio,
}

impl PageKind {
    pub fn label(self) -> &'static str {
        match self {
            PageKind::Album => "Album",
            PageKind::Artist => "Artist",
            PageKind::Playlist => "Playlist",
            PageKind::Radio => "Radio",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PageDescriptor {
    pub kind: PageKind,
    pub id: String,
    /// Radio pages carry their seed kind alongside the seed id.
    pub radio_seed_kind: Option<String>,
}

impl PageDescriptor {
    pub fn new(kind: PageKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            radio_seed_kind: None,
        }
    }

    pub fn radio(seed_kind: impl Into<String>, seed_id: impl Into<String>) -> Self {
        Self {
            kind: PageKind::Radio,
            id: seed_id.into(),
            radio_seed_kind: Some(seed_kind.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageHeader {
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub cover_url: String,
    /// Cover id for display-copy requests.
    pub cover_art: String,
    /// User star count (0..=5) for the one-line page phrase.
    pub rating: i64,
    pub starred: bool,
}

#[derive(Debug, Clone)]
pub struct PageState {
    pub descriptor: PageDescriptor,
    pub header: PageHeader,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: f32,
    pub loading: bool,
    pub error: Option<String>,
    pub selection: Selection,
    /// Container state for the superior bar's contextual chips (Album/Artist).
    pub container_cached: bool,
    pub container_downloading: bool,
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
                cover_art: String::new(),
                rating: 0,
                starred: false,
            },
            rows: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            loading: true,
            error: None,
            selection: Selection::default(),
            container_cached: false,
            container_downloading: false,
        }
    }

    pub fn select(&mut self, index: usize) {
        if self.rows.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = index.min(self.rows.len() - 1);
        }
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
        self.places.truncate(self.position.saturating_add(1));
        self.places.push(descriptor);
        if self.places.len() > 32 {
            self.places.remove(0);
        }
        self.position = self.places.len().saturating_sub(1);
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
        assert_eq!(page.cursor, 0);
        page.rows = vec![Row {
            track: String::new(),
            id: "s".into(),
            kind: RowKind::Song,
            title: "S".into(),
            subtitle: String::new(),
            meta: String::new(),
            cover_url: String::new(),
            song_id: "s".into(),
            album_id: "a".into(),
            starred: false,
            rating: 0,
                    downloaded: false,
        }];
        page.select(9);
        assert_eq!(page.cursor, 0);
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
}

#[cfg(test)]
mod selection_tests {
    use super::*;

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
