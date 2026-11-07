use super::common::*;
use crate::api::*;

#[test]
fn parses_single_object_into_list() {
    let body: Value = serde_json::from_str(
            r#"{"indexes":{"ignoredArticles":"The","lastModified":1,"index":{"name":"A","artist":{"id":"1","name":"AC/DC"}}}}"#,
        )
        .unwrap();

    let indexes: Indexes = extract(body, "indexes").unwrap();
    assert_eq!(indexes.index.len(), 1);
    assert_eq!(indexes.index[0].artist.len(), 1);
    assert_eq!(indexes.index[0].artist[0].name, "AC/DC");
}

#[test]
fn parses_song_type_field_and_missing_fields() {
    let body: Value = serde_json::from_str(
        r#"{"song":{"id":"42","isDir":false,"title":"Hello","artist":"Adele","type":"music"}}"#,
    )
    .unwrap();

    let song: Child = extract(body, "song").unwrap();
    assert_eq!(song.id, "42");
    assert_eq!(song.title, "Hello");
    assert_eq!(song.media_type.as_deref(), Some("music"));
    assert_eq!(song.duration, None);
}

#[test]
fn absent_element_yields_default() {
    let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
    let lyrics: Lyrics = extract(body, "lyrics").unwrap();
    assert!(lyrics.value.is_empty());
}

#[test]
fn album_with_songs_flattens_album_fields() {
    let body: Value = serde_json::from_str(
            r#"{"album":{"id":"a1","name":"Album","songCount":1,"duration":30,"created":"2020-01-01T00:00:00Z","song":[{"id":"s1","isDir":false,"title":"Track"}]}}"#,
        )
        .unwrap();

    let album: AlbumWithSongsID3 = extract(body, "album").unwrap();
    assert_eq!(album.album.name, "Album");
    assert_eq!(album.song.len(), 1);
    assert_eq!(album.song[0].title, "Track");
}

#[test]
fn parses_playlists_and_entries() {
    // A single playlist object becomes a one-element list; fields map.
    let body: Value = serde_json::from_str(
            r#"{"playlists":{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"owner":"me","public":true,"changed":"2026-01-01T00:00:00Z"}}}"#,
        )
        .unwrap();
    let lists: Playlists = extract(body, "playlists").unwrap();
    assert_eq!(lists.playlist.len(), 1);
    assert_eq!(lists.playlist[0].name, "Mix");
    assert_eq!(lists.playlist[0].song_count, Some(2));
    assert_eq!(lists.playlist[0].public, Some(true));

    // The entries flatten into the playlist and keep their order.
    let body: Value = serde_json::from_str(
            r#"{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"entry":[{"id":"s1","isDir":false,"title":"A"},{"id":"s2","isDir":false,"title":"B"}]}}"#,
        )
        .unwrap();
    let page: PlaylistWithSongs = extract(body, "playlist").unwrap();
    assert_eq!(page.entry.len(), 2);
    assert_eq!(page.entry[0].title, "A");

    // An absent element defaults to empty, never errors.
    let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
    let lists: Playlists = extract(body, "playlists").unwrap();
    assert!(lists.playlist.is_empty());
}

#[test]
fn parses_similar_songs() {
    let body: Value = serde_json::from_str(
            r#"{"similarSongs":{"song":[{"id":"s1","isDir":false,"title":"A","artist":"X"},{"id":"s2","isDir":false,"title":"B"}]}}"#,
        )
        .unwrap();
    let similar: SimilarSongs = extract(body, "similarSongs").unwrap();
    assert_eq!(similar.song.len(), 2);
    assert_eq!(similar.song[0].id, "s1");
    assert_eq!(similar.song[0].artist.as_deref(), Some("X"));

    // A single song object is accepted as a one-element list; an absent
    // wrapper is empty, never an error.
    let body: Value =
        serde_json::from_str(r#"{"similarSongs":{"song":{"id":"s1","isDir":false,"title":"A"}}}"#)
            .unwrap();
    let similar: SimilarSongs = extract(body, "similarSongs").unwrap();
    assert_eq!(similar.song.len(), 1);
    let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
    let similar: SimilarSongs = extract(body, "similarSongs").unwrap();
    assert!(similar.song.is_empty());
}

#[test]
fn query_skips_none() {
    let params = query(&[("a", Some("1".to_string())), ("b", None)]);
    assert_eq!(params, vec![("a", "1".to_string())]);
}
