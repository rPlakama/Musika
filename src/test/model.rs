//! `model` wire vocabularies: every surface inherits these spellings.

use crate::model::{PageKind, RowKind};

#[test]
fn row_kind_wire_strings_round_trip() {
    for kind in [
        RowKind::Song,
        RowKind::Album,
        RowKind::Artist,
        RowKind::Playlist,
    ] {
        assert_eq!(RowKind::from_api_kind(kind.api_kind()), Some(kind));
        // The clipboard is the api vocabulary minus playlists.
        assert_eq!(
            kind.clipboard_kind(),
            (kind != RowKind::Playlist).then(|| kind.api_kind())
        );
    }
    assert_eq!(RowKind::Song.api_kind(), "song");
    assert_eq!(RowKind::Song.suggestion_kind(), "track");
    assert_eq!(RowKind::Album.suggestion_kind(), "album");
    assert_eq!(RowKind::Playlist.suggestion_kind(), "playlist");
    // The search/browse payloads name tracks `track`.
    assert_eq!(RowKind::from_api_kind("track"), Some(RowKind::Song));
    assert_eq!(RowKind::from_api_kind("nope"), None);
}

#[test]
fn page_kind_wire_strings_round_trip() {
    for kind in [
        PageKind::Album,
        PageKind::Artist,
        PageKind::Playlist,
        PageKind::Radio,
    ] {
        assert_eq!(PageKind::from_api_kind(kind.api_kind()), Some(kind));
    }
    assert_eq!(PageKind::Album.api_kind(), "album");
    assert_eq!(PageKind::Radio.api_kind(), "radio");
    assert_eq!(PageKind::from_api_kind("song"), None);
}

#[test]
fn page_descriptor_round_trips_the_stored_status_parts() {
    for kind in [
        PageKind::Album,
        PageKind::Artist,
        PageKind::Playlist,
        PageKind::Radio,
    ] {
        let descriptor = crate::model::PageDescriptor::new(kind, "x9");
        let restored =
            crate::model::PageDescriptor::from_stored_parts(kind.api_kind(), &descriptor.id)
                .expect("stored parts restore");
        assert_eq!(restored, descriptor);
    }
    // Stored radio rows do not carry the seed kind; the descriptor defaults it.
    let radio = crate::model::PageDescriptor::from_stored_parts("radio", "seed1")
        .expect("radio restores");
    assert_eq!(radio.kind, PageKind::Radio);
    assert_eq!(radio.radio_seed_kind, None);
    // No page / unknown kind is the "first launch" case.
    assert!(crate::model::PageDescriptor::from_stored_parts("album", "").is_none());
    assert!(crate::model::PageDescriptor::from_stored_parts("bogus", "x").is_none());
}
