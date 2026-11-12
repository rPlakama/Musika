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
    let radio =
        crate::model::PageDescriptor::from_stored_parts("radio", "seed1").expect("radio restores");
    assert_eq!(radio.kind, PageKind::Radio);
    assert_eq!(radio.radio_seed_kind, None);
    // No page / unknown kind is the "first launch" case.
    assert!(crate::model::PageDescriptor::from_stored_parts("album", "").is_none());
    assert!(crate::model::PageDescriptor::from_stored_parts("bogus", "x").is_none());
}

#[test]
fn item_kind_owns_the_canonical_vocabulary() {
    for kind in [
        crate::model::ItemKind::Song,
        crate::model::ItemKind::Album,
        crate::model::ItemKind::Artist,
        crate::model::ItemKind::Playlist,
        crate::model::ItemKind::Radio,
    ] {
        assert_eq!(
            crate::model::ItemKind::from_api_kind(kind.api_kind()),
            Some(kind)
        );
    }
    assert_eq!(
        crate::model::ItemKind::from_api_kind("track"),
        Some(crate::model::ItemKind::Song)
    );
    assert_eq!(crate::model::ItemKind::from_api_kind("nope"), None);
    assert_eq!(crate::model::ItemKind::Song.suggestion_kind(), "track");
    assert_eq!(crate::model::ItemKind::Album.suggestion_kind(), "album");

    // The narrow surface enums are adapters, not restatements.
    for row in [
        RowKind::Song,
        RowKind::Album,
        RowKind::Artist,
        RowKind::Playlist,
    ] {
        assert_eq!(RowKind::from_api_kind(row.api_kind()), Some(row));
        assert_eq!(row.api_kind(), row.item_kind().api_kind());
        assert_eq!(RowKind::from_item_kind(row.item_kind()), Some(row));
    }
    assert_eq!(RowKind::from_api_kind("radio"), None);
    for page in [
        PageKind::Album,
        PageKind::Artist,
        PageKind::Playlist,
        PageKind::Radio,
    ] {
        assert_eq!(PageKind::from_api_kind(page.api_kind()), Some(page));
        assert_eq!(page.api_kind(), page.item_kind().api_kind());
        assert_eq!(page.label(), page.item_kind().label());
        assert_eq!(PageKind::from_item_kind(page.item_kind()), Some(page));
    }
    assert_eq!(PageKind::from_api_kind("track"), None);

    // Command scopes adapt to the same vocabulary.
    use crate::ipc_command::{DownloadScope, SuggestScope};
    assert_eq!(
        DownloadScope::Track.item_kind(),
        crate::model::ItemKind::Song
    );
    assert_eq!(
        SuggestScope::Album.item_kind(),
        Some(crate::model::ItemKind::Album)
    );
    assert_eq!(SuggestScope::Any.item_kind(), None);
}

#[test]
fn track_flattens_the_shared_identity_on_the_wire() {
    let track = crate::player::Track {
        item: crate::model::ItemMeta {
            id: "s1".to_string(),
            title: "Song".to_string(),
            album_id: "al1".to_string(),
            cover_url: "file:///c".to_string(),
            cover_remote: String::new(),
        },
        artist: "Artist".to_string(),
        album: "Album".to_string(),
        favorite: true,
        duration: 12.0,
        url: "file:///s1.flac".to_string(),
        cover_art: "cov1".to_string(),
    };
    let json = serde_json::to_value(&track).unwrap();
    assert_eq!(json["id"], "s1");
    assert_eq!(json["title"], "Song");
    assert_eq!(json["album_id"], "al1");
    assert_eq!(json["cover_url"], "file:///c");
    assert!(json.get("item").is_none(), "identity stays flattened");
    // Field reads go through the shared identity.
    assert_eq!(track.id, "s1");
    assert_eq!(track.album_id, "al1");

    // Payloads serialized before album_id/cover_remote existed still parse.
    let legacy = serde_json::json!({
        "id": "s2",
        "title": "Old",
        "artist": "Artist",
        "album": "Album",
        "favorite": false,
        "duration": 1.0,
        "url": "file:///s2.flac",
        "cover_art": "",
        "cover_url": "",
    });
    let track: crate::player::Track = serde_json::from_value(legacy).unwrap();
    assert_eq!(track.id, "s2");
    assert_eq!(track.album_id, "");
    assert_eq!(track.cover_remote, "");
}

#[test]
fn row_flattens_the_shared_identity_on_the_wire() {
    let row = crate::model::Row {
        item: crate::model::ItemMeta {
            id: "al1".to_string(),
            title: "Album".to_string(),
            album_id: "al1".to_string(),
            cover_url: "file:///al1".to_string(),
            cover_remote: String::new(),
        },
        kind: RowKind::Album,
        subtitle: "Artist · 2026".to_string(),
        meta: "10 tracks".to_string(),
        track: String::new(),
        song_id: String::new(),
        starred: true,
        rating: 3,
    };
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(json["id"], "al1");
    assert_eq!(json["title"], "Album");
    assert_eq!(json["album_id"], "al1");
    assert_eq!(json["cover_url"], "file:///al1");
    assert_eq!(json["kind"], "Album");
    assert!(json.get("item").is_none(), "identity stays flattened");
    // Field reads go through the shared identity.
    assert_eq!(row.id, "al1");
    assert_eq!(row.item.album_id, "al1");
    let back: crate::model::Row = serde_json::from_value(json).unwrap();
    assert_eq!(back.item, row.item);
    assert_eq!(back.kind, row.kind);
}
