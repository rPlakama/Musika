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

/// N-06: one song can be listed under two albums in a single sync. The id is
/// the `_sync_new_songs` primary key, so the keep-set insert used to abort the
/// whole transaction with a UNIQUE violation.
#[tokio::test]
async fn duplicate_song_listing_does_not_break_the_sync() {
    let pool = pool().await;
    let artists = vec![artist("ar1", "Artist", 2)];
    let albums = vec![
        album("al1", "ar1", "First", 1, 100),
        album("al2", "ar1", "Second", 1, 100),
    ];
    let listed_twice = song("s1", "al1", "One", 1);
    let songs = fetched(&[
        ("al1", vec![listed_twice.clone()]),
        ("al2", vec![listed_twice]),
    ]);

    let changes = apply(&pool, &artists, &albums, &songs, 1, true)
        .await
        .unwrap();
    assert_eq!(changes.songs, vec!["s1".to_string()]);
    assert_eq!(counts(&pool).await.unwrap(), (1, 2, 1));
}

/// N-16: a shallow pass that re-fetched no album and found nothing to write is
/// a no-op. It must not bump `library_rev`, because that revision is what lets
/// the post-sync FTS/catalog rebuilds and the mirror rewrite short-circuit.
#[tokio::test]
async fn shallow_noop_pass_keeps_the_library_revision() {
    let pool = pool().await;
    let artists = vec![artist("ar1", "Artist", 1)];
    let albums = vec![album("al1", "ar1", "First", 1, 100)];
    let songs = fetched(&[("al1", vec![song("s1", "al1", "One", 1)])]);
    let first = apply(&pool, &artists, &albums, &songs, 1, true)
        .await
        .unwrap();
    assert!(first.any());
    let revision = crate::library::library_revision(&pool)
        .await
        .unwrap()
        .expect("a real sync records the revision");

    let changes = apply(&pool, &artists, &albums, &HashMap::new(), 2, false)
        .await
        .unwrap();
    assert!(!changes.any());
    assert_eq!(
        crate::library::library_revision(&pool).await.unwrap(),
        Some(revision),
        "a no-op pass must not invalidate the derived caches"
    );
    // The sync bookkeeping still advances so "last synced" stays truthful.
    assert_eq!(crate::library::last_modified(&pool).await.unwrap(), 2);
}

/// N-16: the no-op fast path must still import server-owned fields that the
/// metadata hash omits (album rating/favourite, artist favourite). The row is
/// rewritten even though the metadata diff reports no change.
#[tokio::test]
async fn shallow_pass_imports_server_stars_without_a_metadata_change() {
    let pool = pool().await;
    let artists = vec![artist("ar1", "Artist", 1)];
    let albums = vec![album("al1", "ar1", "First", 1, 100)];
    let songs = fetched(&[("al1", vec![song("s1", "al1", "One", 1)])]);
    apply(&pool, &artists, &albums, &songs, 1, true)
        .await
        .unwrap();

    let starred_at = "2026-02-02T00:00:00Z";
    let mut starred_album = album("al1", "ar1", "First", 1, 100);
    starred_album.starred = Some(starred_at.into());
    starred_album.user_rating = Some(4);
    let mut starred_artist = artist("ar1", "Artist", 1);
    starred_artist.starred = Some(starred_at.into());
    let changes = apply(
        &pool,
        &[starred_artist],
        &[starred_album],
        &HashMap::new(),
        2,
        false,
    )
    .await
    .unwrap();

    // No metadata changed, so the derived-cache invalidation stays quiet…
    assert!(!changes.any());
    // …but the server-owned fields still land in the local rows.
    assert_eq!(
        crate::library::album(&pool, "al1")
            .await
            .unwrap()
            .unwrap()
            .starred
            .as_deref(),
        Some(starred_at)
    );
    assert_eq!(crate::library::album_rating(&pool, "al1").await.unwrap(), 4);
    assert_eq!(
        crate::library::artist(&pool, "ar1")
            .await
            .unwrap()
            .unwrap()
            .starred
            .as_deref(),
        Some(starred_at)
    );
}

/// N-16: a pre-revision database (upgraded install) must still get a revision
/// from its first pass, even a no-op one — otherwise the derived caches could
/// never start short-circuiting.
#[tokio::test]
async fn noop_pass_seeds_the_revision_on_a_pre_revision_database() {
    let pool = pool().await;
    assert_eq!(crate::library::library_revision(&pool).await.unwrap(), None);
    let changes = apply(&pool, &[], &[], &HashMap::new(), 1, false)
        .await
        .unwrap();
    assert!(!changes.any());
    assert_eq!(
        crate::library::library_revision(&pool).await.unwrap(),
        Some(1)
    );
    // A second no-op pass must leave it alone.
    apply(&pool, &[], &[], &HashMap::new(), 2, false)
        .await
        .unwrap();
    assert_eq!(
        crate::library::library_revision(&pool).await.unwrap(),
        Some(1)
    );
}
