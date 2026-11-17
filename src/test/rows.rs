use super::common::*;

#[tokio::test]
async fn counts_and_windows_an_ordered_surface() {
    let library = library_pool().await;
    let media = media_pool().await;
    sqlx::query("INSERT INTO artists (id, name, album_count, cover_art, starred) VALUES ('ar1', 'Artist', 1, '', NULL)")
            .execute(&library).await.unwrap();
    sqlx::query("INSERT INTO albums (id, artist_id, name, artist, year, song_count, duration, cover_art) VALUES ('al1', 'ar1', 'Album', 'Artist', 2026, 1, 120, '')")
            .execute(&library).await.unwrap();
    sqlx::query("INSERT INTO songs (id, title, artist, album, album_id, track, duration, suffix, starred, cover_art) VALUES ('s1', 'Song', 'Artist', 'Album', 'al1', 1, 120, 'flac', NULL, '')")
            .execute(&library).await.unwrap();
    let dbs = dbs_with_media(&library, &media);
    assert_eq!(
        crate::rows::count(&dbs, "albums", "", "Artist", false)
            .await
            .unwrap(),
        1
    );
    let rows = crate::rows::window(&dbs, "albums", "", "Artist", 0, 10, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, "al1");
    assert_eq!(row.kind, crate::model::RowKind::Album);
    assert_eq!(row.title, "Album");
    assert_eq!(row.subtitle, "Artist · 2026");
    assert_eq!(row.meta, "1 tracks");
    assert_eq!(row.album_id, "al1");
}

#[tokio::test]
async fn downloaded_order_filters_every_scope_before_windowing() {
    let _guard = lock_offline_catalog().await;
    crate::offline::clear_cache();
    let library = library_pool().await;
    let media = media_pool().await;
    let download = download_pool().await;
    for (id, name) in [("ar1", "Alpha"), ("ar2", "Beta")] {
        sqlx::query("INSERT INTO artists (id, name, album_count) VALUES (?1, ?2, 1)")
            .bind(id)
            .bind(name)
            .execute(&library)
            .await
            .unwrap();
    }
    for (id, artist_id, name, artist) in [
        ("al1", "ar1", "First", "Alpha"),
        ("al2", "ar2", "Second", "Beta"),
        ("al3", "ar2", "Third", "Beta"),
    ] {
        sqlx::query(
            "INSERT INTO albums (id, artist_id, name, artist, year, song_count, duration)
             VALUES (?1, ?2, ?3, ?4, 2026, 1, 120)",
        )
        .bind(id)
        .bind(artist_id)
        .bind(name)
        .bind(artist)
        .execute(&library)
        .await
        .unwrap();
    }
    for (id, artist, artist_id, album, album_id, track) in [
        ("s1", "Alpha", "ar1", "First", "al1", 1),
        ("s2", "Beta", "ar2", "Second", "al2", 1),
        ("s3", "Beta", "ar2", "Third", "al3", 1),
        ("s4", "Beta", "ar2", "Second", "al2", 2),
    ] {
        sqlx::query(
            "INSERT INTO songs (id, title, artist, artist_id, album, album_id, track, duration)
             VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, 60)",
        )
        .bind(id)
        .bind(artist)
        .bind(artist_id)
        .bind(album)
        .bind(album_id)
        .bind(track)
        .execute(&library)
        .await
        .unwrap();
    }
    // Two finished downloads (s1, s3); s4 is queued, s2 untouched.
    for id in ["s1", "s3"] {
        crate::download::set_state(&download, id, crate::download::DONE, None)
            .await
            .unwrap();
    }
    crate::download::mark_queued(&download, "s4", None)
        .await
        .unwrap();

    let dbs = dbs_with_download(&library, &media, &download);
    assert_eq!(
        crate::rows::count(&dbs, "songs", "", "Downloaded", false)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        crate::rows::count(&dbs, "albums", "", "Downloaded", false)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        crate::rows::count(&dbs, "artists", "", "Downloaded", false)
            .await
            .unwrap(),
        2
    );

    let song_ids = |rows: Vec<crate::model::Row>| -> Vec<String> {
        rows.into_iter().map(|row| row.song_id).collect()
    };
    // Filtering happens before paging: a one-row window at offset 1 still
    // returns the second download, not an empty tail full of holes.
    assert_eq!(
        song_ids(
            crate::rows::window(&dbs, "songs", "", "Downloaded", 1, 1, false)
                .await
                .unwrap()
        ),
        ["s3"]
    );
    let album_ids = |rows: Vec<crate::model::Row>| -> Vec<String> {
        rows.into_iter().map(|row| row.item.id).collect()
    };
    assert_eq!(
        album_ids(
            crate::rows::window(&dbs, "albums", "", "Downloaded", 0, 10, false)
                .await
                .unwrap()
        ),
        ["al1", "al3"]
    );

    // Without the filter the full library is still served.
    assert_eq!(
        crate::rows::count(&dbs, "songs", "", "Artist", false)
            .await
            .unwrap(),
        4
    );
}
