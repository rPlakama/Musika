use super::common::*;

/// ARC-RS-1: an album seed resolves to its first song and reports the
/// album identity even when the page is offline (empty rows).
#[tokio::test]
async fn offline_page_uses_the_album_seed_identity() {
    let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
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
    .execute(&pool)
    .await
    .unwrap();

    let (header, rows) = crate::radio::page(&dbs_on(&pool), "album", "al1", true)
        .await
        .unwrap();
    assert_eq!(header.title, "Kid A");
    assert_eq!(header.subtitle, "Radio needs a connection");
    assert_eq!(header.meta, "Similar songs");
    assert!(header.collage.is_empty());
    assert!(rows.is_empty());

    assert!(
        crate::radio::page(&dbs_on(&pool), "album", "missing", true)
            .await
            .is_err()
    );
}

/// ARC-RS-1: a song row seeds radio directly. A local song reports its
/// own identity; a server-only row (radio/playlist rows) still seeds by
/// id instead of failing the local lookup.
#[tokio::test]
async fn song_seed_uses_local_identity_or_falls_back_to_the_id() {
    let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    sqlx::raw_sql(
            "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Radiohead', 1);
             INSERT INTO albums (id, artist_id, name, artist, song_count)
                 VALUES ('al1', 'ar1', 'Kid A', 'Radiohead', 1);
             INSERT INTO songs
                 (id, album_id, title, artist, album, track, disc_number, duration, play_count)
             VALUES ('s1', 'al1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 240, 0);",
        )
        .execute(&pool)
        .await
        .unwrap();

    let (local, _) = crate::radio::page(&dbs_on(&pool), "song", "s1", true)
        .await
        .unwrap();
    assert_eq!(local.title, "Everything In Its Right Place");

    let (remote, _) = crate::radio::page(&dbs_on(&pool), "song", "server-only", true)
        .await
        .unwrap();
    assert_eq!(remote.title, "Radio");
    assert_eq!(remote.subtitle, "Radio needs a connection");
}
