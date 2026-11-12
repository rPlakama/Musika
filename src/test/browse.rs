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
