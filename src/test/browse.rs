use super::common::*;
use crate::browse::{merge_hits, search_results_index};

/// ARC-SEARCH: the Nucleo engine answers from the in-memory catalog and
/// keeps S-8 (one album match = track order) and S-9 (Most played).
#[tokio::test]
async fn nucleo_search_payload_follows_the_one_album_and_most_played() {
    let library = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    let covers = crate::test::memory_pool(sqlx::migrate!("./migrations_covers")).await;
    let dbs = crate::db::Dbs {
        credentials: library.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers,
        media: library.clone(),
        lyrics: library.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: library.clone(),
        playlist: library.clone(),
    };
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Radiohead', 1);
             INSERT INTO albums (id, artist_id, name, artist, song_count)
                 VALUES ('al1', 'ar1', 'Kid A', 'Radiohead', 2);
             INSERT INTO songs
                 (id, album_id, title, artist, album, track, disc_number, duration, play_count)
             VALUES
                 ('s1', 'al1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 240, 0),
                 ('s2', 'al1', 'Idioteque', 'Radiohead', 'Kid A', 2, 1, 300, 5);",
    )
    .execute(&dbs.library)
    .await
    .unwrap();

    let results = search_results_index(&dbs, "rdo", false, 20).await.unwrap();
    assert!(!results.most_played);
    assert_eq!(results.album_context.as_deref(), Some("al1"));
    let ids: Vec<&str> = results.songs.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, ["s1", "s2"], "one album match reads in track order");

    let results = search_results_index(&dbs, "", false, 20).await.unwrap();
    assert!(results.most_played);
    assert!(results.album_context.is_none());
    assert_eq!(results.songs[0].id, "s2");
    // The unified search browses every scope on an empty query.
    assert_eq!(results.artists[0].id, "ar1");
    assert_eq!(results.albums[0].id, "al1");

    crate::search::index::clear();
}

#[test]
fn hybrid_merge_keeps_primary_order_and_dedupes() {
    use crate::browse::search::SearchArtist;
    let mut primary = vec![
        SearchArtist {
            id: "a".to_string(),
            ..SearchArtist::default()
        },
        SearchArtist {
            id: "b".to_string(),
            ..SearchArtist::default()
        },
    ];
    let secondary = vec![
        SearchArtist {
            id: "b".to_string(),
            ..SearchArtist::default()
        },
        SearchArtist {
            id: "c".to_string(),
            ..SearchArtist::default()
        },
        SearchArtist {
            id: "d".to_string(),
            ..SearchArtist::default()
        },
    ];
    merge_hits(&mut primary, secondary, 3, |row| &row.id);
    let ids: Vec<&str> = primary.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

#[test]
fn subtitles_drop_missing_parts_and_their_separator() {
    use crate::browse::rows::{album_subtitle, song_subtitle_from_parts};
    use crate::library::AlbumRow;

    let album = |artist: Option<&str>, year: Option<i32>| AlbumRow {
        id: "al".to_string(),
        title: "Album".to_string(),
        artist: artist.map(str::to_string),
        year,
        song_count: 1,
        duration: 60,
        cover_art: None,
        starred: None,
        rating: 0,
    };

    // N-21: an empty artist must not leave a leading " · ".
    assert_eq!(
        album_subtitle(&album(Some("Artist"), Some(2026))),
        "Artist · 2026"
    );
    assert_eq!(album_subtitle(&album(Some("Artist"), None)), "Artist");
    assert_eq!(
        album_subtitle(&album(None, Some(2026))),
        "2026",
        "no leading separator for an artist-less album"
    );
    assert_eq!(album_subtitle(&album(Some("  "), Some(2026))), "2026");
    assert_eq!(album_subtitle(&album(None, None)), "");

    assert_eq!(
        song_subtitle_from_parts(Some("Artist"), Some("Album")),
        "Artist · Album"
    );
    assert_eq!(song_subtitle_from_parts(None, Some("Album")), "Album");
    assert_eq!(song_subtitle_from_parts(Some("Artist"), None), "Artist");
    assert_eq!(song_subtitle_from_parts(Some(""), Some("Album")), "Album");
    assert_eq!(
        song_subtitle_from_parts(Some("Artist"), Some("  ")),
        "Artist"
    );
    assert_eq!(song_subtitle_from_parts(None, None), "");
}

/// N-08: offline unified search must not list a playlist whose entries are all
/// server-only, and N-20: the per-page artist state comes from the artist_user
/// overlay (rating/starred) instead of the whole-table snapshot.
#[tokio::test]
async fn offline_search_filters_server_playlists_and_reads_artist_state() {
    let _offline = lock_offline_catalog().await;
    crate::offline::clear_cache();
    let mut dbs = isolated_dbs().await;
    dbs.search = crate::test::memory_pool(sqlx::migrate!("./migrations_search")).await;
    dbs.playlist = crate::test::memory_pool(None).await;
    dbs.artist_user = crate::test::memory_pool(None).await;
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Boards of Canada', 1);
         INSERT INTO albums (id, artist_id, name, artist, song_count)
             VALUES ('al1', 'ar1', 'Music Has the Right', 'Boards of Canada', 2);
         INSERT INTO songs (id, album_id, artist_id, title, artist, album, track, duration)
             VALUES
                 ('s1', 'al1', 'ar1', 'Roygbiv', 'Boards of Canada', 'Music Has the Right', 1, 180),
                 ('s2', 'al1', 'ar1', 'Olson', 'Boards of Canada', 'Music Has the Right', 2, 90);",
    )
    .execute(&dbs.library)
    .await
    .unwrap();
    // A local playlist with one downloaded entry; a server playlist with none.
    sqlx::raw_sql(
        "CREATE TABLE playlist (
             id TEXT PRIMARY KEY,
             source TEXT NOT NULL,
             name TEXT NOT NULL,
             song_count INTEGER NOT NULL DEFAULT 0,
             duration INTEGER NOT NULL DEFAULT 0,
             owner TEXT,
             cover_art TEXT
         );
         CREATE TABLE playlist_song (
             playlist_id TEXT NOT NULL,
             position INTEGER NOT NULL,
             song_id TEXT,
             cover_art TEXT
         );
         INSERT INTO playlist (id, source, name, song_count) VALUES
             ('pl-downloaded', 'subsonic', 'Campfire mix', 1),
             ('pl-server', 'subsonic', 'Campfire server only', 1);
         INSERT INTO playlist_song (playlist_id, position, song_id) VALUES
             ('pl-downloaded', 0, 's1'),
             ('pl-server', 0, 'missing-song');",
    )
    .execute(&dbs.playlist)
    .await
    .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE artist_user (
             artist_id TEXT PRIMARY KEY,
             starred INTEGER NOT NULL DEFAULT 0,
             rating INTEGER NOT NULL DEFAULT 0,
             updated_at TEXT NOT NULL
         );
         INSERT INTO artist_user (artist_id, starred, rating, updated_at)
             VALUES ('ar1', 1, 5, '1');",
    )
    .execute(&dbs.artist_user)
    .await
    .unwrap();
    crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
        .await
        .unwrap();
    let mut tx = dbs.search.begin().await.unwrap();
    crate::search::insert_search_rows(
        &mut tx,
        &[
            crate::library::SongSearchRow {
                id: "s1".to_string(),
                title: "Roygbiv".to_string(),
                artist: Some("Boards of Canada".to_string()),
                album: Some("Music Has the Right".to_string()),
                duration: Some(180),
                play_count: Some(0),
            },
            crate::library::SongSearchRow {
                id: "s2".to_string(),
                title: "Olson".to_string(),
                artist: Some("Boards of Canada".to_string()),
                album: Some("Music Has the Right".to_string()),
                duration: Some(90),
                play_count: Some(0),
            },
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let results = crate::browse::search::search_results_fts(&dbs, "", true, 20)
        .await
        .unwrap();
    assert!(results.most_played);
    let playlist_ids: Vec<&str> = results
        .playlists
        .iter()
        .map(|playlist| playlist.id.as_str())
        .collect();
    assert_eq!(
        playlist_ids,
        ["pl-downloaded"],
        "a server-only playlist must not leak into offline search"
    );
    assert_eq!(
        results.artists[0].rating, 5,
        "per-page artist state is decorated from the overlay"
    );
    assert!(results.artists[0].starred);

    let song_ids: Vec<&str> = results.songs.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(
        song_ids,
        ["s1"],
        "offline search keeps only downloaded songs"
    );
    crate::offline::clear_cache();
}
