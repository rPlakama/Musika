use std::collections::HashMap;

use crate::api::{AlbumID3, ArtistID3, Child};
use crate::library::{album_hash, album_manifest, apply, counts, full_sync_due};
use sqlx::sqlite::SqlitePool;

async fn pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations_library")
        .run(&pool)
        .await
        .unwrap();
    pool
}

fn artist(id: &str, name: &str, album_count: i32) -> ArtistID3 {
    ArtistID3 {
        id: id.into(),
        name: name.into(),
        album_count,
        ..ArtistID3::default()
    }
}

fn album(id: &str, artist_id: &str, name: &str, song_count: i32, duration: i32) -> AlbumID3 {
    AlbumID3 {
        id: id.into(),
        name: name.into(),
        artist_id: Some(artist_id.into()),
        artist: Some("Artist".into()),
        song_count,
        duration,
        ..AlbumID3::default()
    }
}

fn song(id: &str, album_id: &str, title: &str, track: i32) -> Child {
    Child {
        id: id.into(),
        album_id: Some(album_id.into()),
        artist_id: Some("ar1".into()),
        title: title.into(),
        album: Some("Album".into()),
        artist: Some("Artist".into()),
        track: Some(track),
        duration: Some(100),
        ..Child::default()
    }
}

fn fetched(pairs: &[(&str, Vec<Child>)]) -> HashMap<String, Vec<Child>> {
    pairs
        .iter()
        .map(|(album, songs)| ((*album).into(), songs.clone()))
        .collect()
}

#[tokio::test]
async fn apply_reconciles_moves_deletions_and_additions() {
    let pool = pool().await;
    let artists = vec![artist("ar1", "Artist", 2)];
    let albums = vec![
        album("al1", "ar1", "First", 2, 200),
        album("al2", "ar1", "Second", 1, 100),
    ];
    let first = fetched(&[
        (
            "al1",
            vec![song("s1", "al1", "One", 1), song("s2", "al1", "Two", 2)],
        ),
        ("al2", vec![song("s3", "al2", "Three", 1)]),
    ]);
    let changes = apply(&pool, &artists, &albums, &first, 1, true)
        .await
        .unwrap();
    assert!(changes.any());
    assert_eq!(counts(&pool).await.unwrap(), (3, 2, 1));
    assert!(!full_sync_due(&pool).await);

    // Upstream move: s1 goes from al1 to al2; both album hashes changed.
    let moved_albums = vec![
        album("al1", "ar1", "First", 1, 100),
        album("al2", "ar1", "Second", 2, 200),
    ];
    let moved = fetched(&[
        ("al1", vec![song("s2", "al1", "Two", 1)]),
        (
            "al2",
            vec![song("s3", "al2", "Three", 1), song("s1", "al2", "One", 2)],
        ),
    ]);
    let changes = apply(&pool, &artists, &moved_albums, &moved, 2, false)
        .await
        .unwrap();
    assert!(changes.any());
    let (album_id, track): (Option<String>, Option<i32>) =
        sqlx::query_as("SELECT album_id, track FROM songs WHERE id = 's1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(album_id.as_deref(), Some("al2"));
    assert_eq!(track, Some(2));
    assert_eq!(counts(&pool).await.unwrap(), (3, 2, 1));

    // Upstream delete: al2 disappears with both of its songs.
    let deleted = apply(
        &pool,
        &artists,
        &[album("al1", "ar1", "First", 1, 100)],
        &fetched(&[("al1", vec![song("s2", "al1", "Two", 1)])]),
        3,
        true,
    )
    .await
    .unwrap();
    assert!(deleted.any());
    assert_eq!(counts(&pool).await.unwrap(), (1, 1, 1));

    // Upstream add: a new artist/album/song appears.
    let added_artists = vec![artist("ar1", "Artist", 2), artist("ar2", "New", 1)];
    let added_albums = vec![
        album("al1", "ar1", "First", 1, 100),
        album("al3", "ar2", "Third", 1, 100),
    ];
    let mut added = fetched(&[("al3", vec![song("s4", "al3", "Four", 1)])]);
    // al1 is unchanged and not fetched; its s2 must survive.
    added.remove("al1");
    apply(&pool, &added_artists, &added_albums, &added, 4, false)
        .await
        .unwrap();
    assert_eq!(counts(&pool).await.unwrap(), (2, 2, 2));
    let album_id: Option<String> = sqlx::query_scalar("SELECT album_id FROM songs WHERE id = 's2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(album_id.as_deref(), Some("al1"));
}

#[tokio::test]
async fn shallow_apply_keeps_untouched_album_songs() {
    let pool = pool().await;
    let artists = vec![artist("ar1", "Artist", 2)];
    let albums = vec![
        album("al1", "ar1", "First", 1, 100),
        album("al2", "ar1", "Second", 1, 100),
    ];
    let all = fetched(&[
        ("al1", vec![song("s1", "al1", "One", 1)]),
        ("al2", vec![song("s2", "al2", "Two", 1)]),
    ]);
    apply(&pool, &artists, &albums, &all, 1, true)
        .await
        .unwrap();
    let manifest = album_manifest(&pool).await.unwrap();
    assert_eq!(manifest.len(), 2);
    assert_eq!(manifest.get("al1").unwrap().hash, album_hash(&albums[0]));

    // Only al1 is fetched (e.g. a changed album); al2 must be untouched.
    let only_al1 = fetched(&[("al1", vec![song("s1", "al1", "One!", 1)])]);
    apply(&pool, &artists, &albums, &only_al1, 2, false)
        .await
        .unwrap();
    assert_eq!(counts(&pool).await.unwrap(), (2, 2, 1));
    let title: String = sqlx::query_scalar("SELECT title FROM songs WHERE id = 's2'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title, "Two");
}
