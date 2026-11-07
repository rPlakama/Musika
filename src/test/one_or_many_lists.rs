use crate::api::extract;

#[test]
fn empty_arrays_are_empty_not_one_default_row() {
    let body = serde_json::json!({
        "indexes": {
            "ignoredArticles": "The",
            "lastModified": 1,
            "shortcut": [],
            "index": []
        }
    });
    let indexes: crate::api::Indexes = extract(body, "indexes").unwrap();
    assert!(indexes.shortcut.is_empty());
    assert!(indexes.index.is_empty());

    let body = serde_json::json!({"artist": {"id": "ar1", "name": "A", "album": []}});
    let artist: crate::api::ArtistWithAlbumsID3 = extract(body, "artist").unwrap();
    assert!(artist.album.is_empty());

    let body = serde_json::json!({"album": {"id": "al1", "name": "B", "song": []}});
    let album: crate::api::AlbumWithSongsID3 = extract(body, "album").unwrap();
    assert!(album.song.is_empty());
}

#[test]
fn single_objects_still_deserialize() {
    let body = serde_json::json!({
        "indexes": {"ignoredArticles": "The", "lastModified": 1,
            "shortcut": {"id": "ar2", "name": "B"},
            "index": {"name": "A", "artist": {"id": "ar1", "name": "A"}}}
    });
    let indexes: crate::api::Indexes = extract(body, "indexes").unwrap();
    assert_eq!(indexes.shortcut.len(), 1);
    assert_eq!(indexes.index.len(), 1);
    assert_eq!(indexes.index[0].artist[0].id, "ar1");
}
