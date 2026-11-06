// Musika — GPUUI shell/list state (M1 first slice).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub kind: RowKind,
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub cover_url: String,
    pub song_id: String,
    pub album_id: String,
    pub starred: bool,
    pub rating: i64,
}

#[derive(Debug, Clone)]
pub struct ViewState {
    pub scope: Scope,
    pub order: usize,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: f32,
    pub loading: bool,
    pub error: Option<String>,
    pub selection: Selection,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            scope: Scope::Albums,
            order: 0,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            loading: true,
            error: None,
            selection: Selection::default(),
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
            rows: vec![Row {
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
            }],
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageKind {
    Album,
    Artist,
    Playlist,
}

impl PageKind {
    pub fn label(self) -> &'static str {
        match self {
            PageKind::Album => "Album",
            PageKind::Artist => "Artist",
            PageKind::Playlist => "Playlist",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PageDescriptor {
    pub kind: PageKind,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct PageHeader {
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub cover_url: String,
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
            },
            rows: Vec::new(),
            cursor: 0,
            scroll: 0.0,
            loading: true,
            error: None,
            selection: Selection::default(),
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

/// Travel log: the QML shell keeps the last 32 places. M1 keeps descriptors
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
    fn travel_log_back_and_forward() {
        let mut travel = Travel::default();
        travel.open(PageDescriptor { kind: PageKind::Album, id: "a".into() });
        travel.open(PageDescriptor { kind: PageKind::Artist, id: "b".into() });
        assert_eq!(travel.back().unwrap().id, "a");
        assert_eq!(travel.step(1).unwrap().id, "b");
        assert!(travel.step(1).is_none());
    }

    #[test]
    fn travel_truncates_forward_history_on_open() {
        let mut travel = Travel::default();
        for id in ["a", "b", "c"] {
            travel.open(PageDescriptor { kind: PageKind::Album, id: id.into() });
        }
        travel.back();
        travel.open(PageDescriptor { kind: PageKind::Album, id: "d".into() });
        assert_eq!(travel.places.len(), 3);
        assert_eq!(travel.current().unwrap().id, "d");
    }
}

#[cfg(test)]
mod page_kind_tests {
    use super::*;

    #[test]
    fn page_select_clamps() {
        let mut page = PageState::new(PageDescriptor { kind: PageKind::Album, id: "a".into() });
        page.select(4);
        assert_eq!(page.cursor, 0);
        page.rows = vec![Row {
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
