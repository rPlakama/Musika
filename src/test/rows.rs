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
    assert!(!row.downloaded);
}
