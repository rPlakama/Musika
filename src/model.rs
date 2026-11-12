//! Shared UI/backend payload types.
//!
//! These were originally defined in `ui::state`; the core daemon needs them
//! without a GPUI window, so they live in a neutral module. Metric-only
//! constants stay in `ui::state`.

use serde::{Deserialize, Serialize};

/// Number of search results a scope can paint; also the backend query cap.
pub const SEARCH_VISIBLE_ROWS: usize = 5;
/// Rows requested from the backend; the overlay scrolls beyond the visible 5.
pub const SEARCH_FETCH_ROWS: usize = 100;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Albums,
    Songs,
    Artists,
    Playlists,
}

impl Scope {
    pub const ALL: [Scope; 4] = [
        Scope::Albums,
        Scope::Songs,
        Scope::Artists,
        Scope::Playlists,
    ];

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

/// The canonical item vocabulary: the five things a row, page or queue entry
/// can name. Each surface keeps its own narrow enum (a library row has no
/// Radio, a `:` command suggestion names tracks) and adapts to this one, so
/// the wire spellings and labels are declared once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Song,
    Album,
    Artist,
    Playlist,
    Radio,
}

impl ItemKind {
    /// The canonical wire spelling (the bridge, daemon and pages all use it).
    pub fn api_kind(self) -> &'static str {
        match self {
            Self::Song => "song",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
            Self::Radio => "radio",
        }
    }

    /// Parse the wire vocabulary; `None` for an unknown value. `track` is
    /// accepted as the search/browse payload spelling of a song.
    pub fn from_api_kind(value: &str) -> Option<Self> {
        match value {
            "song" | "track" => Some(Self::Song),
            "album" => Some(Self::Album),
            "artist" => Some(Self::Artist),
            "playlist" => Some(Self::Playlist),
            "radio" => Some(Self::Radio),
            _ => None,
        }
    }

    /// The display label used by page bars and suggestions.
    pub fn label(self) -> &'static str {
        match self {
            Self::Song => "Song",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Playlist => "Playlist",
            Self::Radio => "Radio",
        }
    }

    /// The command-suggestion vocabulary (`track` instead of `song`).
    pub fn suggestion_kind(self) -> &'static str {
        match self {
            Self::Song => "track",
            _ => self.api_kind(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowKind {
    Album,
    Song,
    Artist,
    Playlist,
}

impl RowKind {
    /// The canonical vocabulary this row belongs to.
    pub fn item_kind(self) -> ItemKind {
        match self {
            Self::Song => ItemKind::Song,
            Self::Album => ItemKind::Album,
            Self::Artist => ItemKind::Artist,
            Self::Playlist => ItemKind::Playlist,
        }
    }

    /// The wire vocabulary shared by the quick actions, the bridge RPCs and
    /// the daemon (`song` is the canonical track spelling at this boundary).
    pub fn api_kind(self) -> &'static str {
        self.item_kind().api_kind()
    }

    /// Parse the wire vocabulary; `None` for an unknown value or a kind a
    /// library row cannot represent (`radio`).
    pub fn from_api_kind(value: &str) -> Option<Self> {
        Self::from_item_kind(ItemKind::from_api_kind(value)?)
    }

    /// The row form of a canonical item kind; `None` for Radio.
    pub fn from_item_kind(kind: ItemKind) -> Option<Self> {
        match kind {
            ItemKind::Song => Some(Self::Song),
            ItemKind::Album => Some(Self::Album),
            ItemKind::Artist => Some(Self::Artist),
            ItemKind::Playlist => Some(Self::Playlist),
            ItemKind::Radio => None,
        }
    }

    /// The row-clipboard kind; playlists are not yankable/queueable.
    pub fn clipboard_kind(self) -> Option<&'static str> {
        (self != Self::Playlist).then(|| self.api_kind())
    }

    /// The command-suggestion vocabulary (`track` instead of `song`).
    pub fn suggestion_kind(self) -> &'static str {
        match self {
            Self::Song => "track",
            _ => self.api_kind(),
        }
    }
}

/// The identity every item carries across playback, presentation and
/// storage: the library id, the display title, the source album id and the
/// resolved cover URLs. Surfacing types embed it (wire-compatible through
/// `#[serde(flatten)]`) and project their own extras on top of it; DB rows
/// convert with [`crate::library::SongRow::item_meta`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemMeta {
    pub id: String,
    pub title: String,
    /// Source album id (empty when the item has none).
    pub album_id: String,
    pub cover_url: String,
    /// Provider URL used when the cached row cover does not load.
    pub cover_remote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub id: String,
    pub kind: RowKind,
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    /// Album track number/module text, searchable by `/` but not painted.
    pub track: String,
    pub cover_url: String,
    /// Provider URL used when the cached row cover does not load.
    pub cover_remote: String,
    pub song_id: String,
    pub album_id: String,
    pub starred: bool,
    pub rating: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageKind {
    Album,
    Artist,
    Playlist,
    Radio,
}

impl PageKind {
    /// The canonical vocabulary this page belongs to.
    pub fn item_kind(self) -> ItemKind {
        match self {
            PageKind::Album => ItemKind::Album,
            PageKind::Artist => ItemKind::Artist,
            PageKind::Playlist => ItemKind::Playlist,
            PageKind::Radio => ItemKind::Radio,
        }
    }

    pub fn label(self) -> &'static str {
        self.item_kind().label()
    }

    /// The wire vocabulary shared by the daemon, the bridge and the UI.
    pub fn api_kind(self) -> &'static str {
        self.item_kind().api_kind()
    }

    /// Parse the wire vocabulary; `None` for an unknown value or a kind no
    /// page can represent (`song`).
    pub fn from_api_kind(value: &str) -> Option<Self> {
        Self::from_item_kind(ItemKind::from_api_kind(value)?)
    }

    /// The page form of a canonical item kind; `None` for Song.
    pub fn from_item_kind(kind: ItemKind) -> Option<Self> {
        match kind {
            ItemKind::Album => Some(PageKind::Album),
            ItemKind::Artist => Some(PageKind::Artist),
            ItemKind::Playlist => Some(PageKind::Playlist),
            ItemKind::Radio => Some(PageKind::Radio),
            ItemKind::Song => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

    /// Rebuild the descriptor from the persisted session status's text fields
    /// (`page_kind`/`page_id`). This is the one place those strings become a
    /// descriptor; the caller writes them back with [`PageKind::api_kind`].
    pub fn from_stored_parts(kind: &str, id: &str) -> Option<Self> {
        if id.is_empty() {
            return None;
        }
        let kind = PageKind::from_api_kind(kind)?;
        Some(Self::new(kind, id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageHeader {
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub cover_url: String,
    /// Provider `getCoverArt` URL used when the cached copy does not load.
    pub cover_remote: String,
    /// Distinct cover URLs for the Playlist/Radio golden-ratio mosaic.
    pub collage: Vec<String>,
    /// Provider fallbacks aligned with [`Self::collage`].
    pub collage_remote: Vec<String>,
    /// Cover ids aligned with [`Self::collage`], used to prefer local hero
    /// caches over streamed provider art.
    pub collage_art: Vec<String>,
    /// Cover id for display-copy requests.
    pub cover_art: String,
    /// User star count (0..=5) for the one-line page phrase.
    pub rating: i64,
    pub starred: bool,
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    #[test]
    fn row_and_page_payloads_round_trip_json() {
        let row = Row {
            id: "s1".into(),
            kind: RowKind::Song,
            title: "Title".into(),
            subtitle: "Artist".into(),
            meta: "Album".into(),
            track: "1".into(),
            cover_url: "file:///cover.jpg".into(),
            cover_remote: "https://example.test/art".into(),
            song_id: "s1".into(),
            album_id: "al1".into(),
            starred: true,
            rating: 3,
        };
        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(serde_json::from_str::<Row>(&json).unwrap(), row);

        let descriptor = PageDescriptor::radio("album", "al1");
        let json = serde_json::to_string(&descriptor).unwrap();
        assert_eq!(
            serde_json::from_str::<PageDescriptor>(&json).unwrap(),
            descriptor
        );
    }
}
