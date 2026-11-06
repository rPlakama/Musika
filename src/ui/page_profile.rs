//! One PageEntry scheme: kind differences are profile variables, not copies.

use crate::ui::scroll_modal::{self, ScrollProfile};
use crate::ui::state::{PageDescriptor, PageKind, Row, RowKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSource {
    Album,
    ArtistAlbums,
    ArtistTracks,
    Playlist,
    Radio { seed_kind: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAction {
    OpenAlbum,
    PlayAlbum,
    PlayArtist,
    PlayPlaylist,
    PlayRadio,
}

#[derive(Debug, Clone)]
pub struct PageProfile {
    pub kind: PageKind,
    pub source: PageSource,
    pub hero_cover: bool,
    pub collage: bool,
    pub notes: bool,
    pub rating: bool,
    pub favorite: bool,
    pub artist_link: bool,
    pub show_row_cover: bool,
    pub empty_text: &'static str,
    pub scroll: ScrollProfile,
}

impl PageProfile {
    pub fn for_descriptor(descriptor: &PageDescriptor) -> Self {
        match descriptor.kind {
            PageKind::Album => Self {
                kind: PageKind::Album,
                source: PageSource::Album,
                hero_cover: true,
                collage: false,
                notes: true,
                rating: true,
                favorite: true,
                artist_link: true,
                show_row_cover: false,
                empty_text: "No tracks in this album",
                scroll: scroll_modal::ALBUM,
            },
            PageKind::Artist => Self {
                kind: PageKind::Artist,
                source: PageSource::ArtistAlbums,
                hero_cover: true,
                collage: false,
                notes: true,
                rating: true,
                favorite: true,
                artist_link: false,
                show_row_cover: true,
                empty_text: "No albums by this artist",
                scroll: scroll_modal::ALBUM,
            },
            PageKind::Playlist => Self {
                kind: PageKind::Playlist,
                source: PageSource::Playlist,
                hero_cover: false,
                collage: true,
                notes: false,
                rating: false,
                favorite: false,
                artist_link: false,
                show_row_cover: true,
                empty_text: "No tracks in this playlist",
                scroll: scroll_modal::ALBUM,
            },
            PageKind::Radio => Self {
                kind: PageKind::Radio,
                source: PageSource::Radio {
                    seed_kind: descriptor
                        .radio_seed_kind
                        .clone()
                        .unwrap_or_else(|| "album".to_string()),
                },
                hero_cover: false,
                collage: true,
                notes: false,
                rating: false,
                favorite: false,
                artist_link: false,
                show_row_cover: true,
                empty_text: "No similar songs found",
                scroll: scroll_modal::ALBUM,
            },
        }
    }

    pub fn row_action(&self, row: &Row) -> PageAction {
        match &self.source {
            PageSource::Album => PageAction::PlayAlbum,
            PageSource::ArtistAlbums => PageAction::OpenAlbum,
            PageSource::ArtistTracks => PageAction::PlayArtist,
            PageSource::Playlist => PageAction::PlayPlaylist,
            PageSource::Radio { .. } => PageAction::PlayRadio,
        }
        .or_row(row, self.kind)
    }
}

impl PageAction {
    fn or_row(self, row: &Row, kind: PageKind) -> PageAction {
        if matches!(kind, PageKind::Artist) && row.kind == RowKind::Album {
            PageAction::OpenAlbum
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::PageDescriptor;

    fn row(kind: RowKind) -> Row {
        Row {
            track: String::new(),
            id: "r".into(),
            kind,
            title: String::new(),
            subtitle: String::new(),
            meta: String::new(),
            cover_url: String::new(),
            song_id: "s".into(),
            album_id: "a".into(),
            starred: false,
            rating: 0,
            downloaded: false,
        }
    }

    #[test]
    fn profile_matrix() {
        let album = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Album, "a"));
        assert!(album.hero_cover && album.notes && album.rating && !album.show_row_cover);
        let artist = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Artist, "ar"));
        assert!(artist.hero_cover && artist.notes && artist.show_row_cover);
        let playlist = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Playlist, "p"));
        assert!(playlist.collage && playlist.show_row_cover && !playlist.notes);
        let radio = PageProfile::for_descriptor(&PageDescriptor::radio("album", "s"));
        assert!(radio.collage && radio.show_row_cover && !radio.favorite);
    }

    #[test]
    fn rows_resolve_actions() {
        let artist = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Artist, "ar"));
        assert_eq!(artist.row_action(&row(RowKind::Album)), PageAction::OpenAlbum);
        let album = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Album, "a"));
        assert_eq!(album.row_action(&row(RowKind::Song)), PageAction::PlayAlbum);
        let playlist = PageProfile::for_descriptor(&PageDescriptor::new(PageKind::Playlist, "p"));
        assert_eq!(playlist.row_action(&row(RowKind::Song)), PageAction::PlayPlaylist);
        let radio = PageProfile::for_descriptor(&PageDescriptor::radio("album", "s"));
        assert_eq!(radio.row_action(&row(RowKind::Song)), PageAction::PlayRadio);
    }
}
