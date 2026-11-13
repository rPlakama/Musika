use super::common::*;

const MINI_LIBRARY: &str = "
    INSERT INTO artists (id, name, album_count) VALUES
        ('ra1', 'Radiohead', 1), ('ra2', 'Portishead', 1);
    INSERT INTO albums (id, artist_id, name, artist, year, genre, song_count) VALUES
        ('ral1', 'ra1', 'Kid A', 'Radiohead', 2000, 'Alternative Rock', 2),
        ('ral2', 'ra2', 'Dummy', 'Portishead', 1994, 'Trip Hop', 2);
    INSERT INTO songs
        (id, album_id, artist_id, title, artist, album, track, disc_number, year, genre, duration)
    VALUES
        ('rs1', 'ral1', 'ra1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 2000, 'Alternative Rock', 240),
        ('rs2', 'ral1', 'ra1', 'Idioteque', 'Radiohead', 'Kid A', 2, 1, 2000, 'Alternative Rock', 300),
        ('rs3', 'ral2', 'ra2', 'Mysterons', 'Portishead', 'Dummy', 1, 1, 1994, 'Trip Hop', 300),
        ('rs4', 'ral2', 'ra2', 'Sour Times', 'Portishead', 'Dummy', 2, 1, 1994, 'Trip Hop', 250);
";

async fn mini_dbs() -> Dbs {
    let dbs = isolated_dbs().await;
    sqlx::raw_sql(MINI_LIBRARY)
        .execute(&dbs.library)
        .await
        .unwrap();
    dbs
}

/// ARC-RS-1: radio is a local expansion now. Offline keeps the seed identity
/// and lists only downloaded songs instead of saying "needs a connection".
#[tokio::test]
async fn offline_page_keeps_seed_identity_and_lists_playable_rows() {
    let _offline = lock_offline_catalog().await;
    let dbs = mini_dbs().await;
    sqlx::query(
        "INSERT INTO download (song_id, state, requested_at, updated_at)
         VALUES ('rs3', 'done', 0, 0)",
    )
    .execute(&dbs.download)
    .await
    .unwrap();
    crate::offline::clear_cache();
    crate::radio::clear_cache();

    let (header, rows) = crate::radio::page(&dbs, "album", "ral1", true)
        .await
        .unwrap();
    assert_eq!(header.title, "Kid A");
    assert_eq!(header.subtitle, "Radio (offline) · Radiohead");
    assert_eq!(header.meta, "Similar songs · local");
    assert_eq!(
        rows.iter()
            .map(|row| row.song_id.as_str())
            .collect::<Vec<_>>(),
        vec!["rs3"],
        "offline radio must only list the downloaded row and keep the seed album out"
    );
    crate::offline::clear_cache();
    crate::radio::clear_cache();
}

/// A server-only row that never made it into the synced library cannot be
/// expanded locally; report it instead of silently using the network.
#[tokio::test]
async fn song_seed_missing_from_the_local_library_is_an_error() {
    let dbs = mini_dbs().await;
    let result = crate::radio::page(&dbs, "song", "server-only", true).await;
    assert!(
        result.is_err(),
        "a missing local seed must fail instead of falling back to HTTP"
    );
}

/// The local expansion must not touch the credentials/client path: radio rows
/// resolve with an empty credentials database (the old path always fetched a
/// `getSimilarSongs` client first).
#[tokio::test]
async fn local_radio_rows_do_not_need_credentials_or_network() {
    let dbs = mini_dbs().await;
    crate::radio::clear_cache();
    let first = crate::radio::radio_rows(&dbs, "album", "ral1")
        .await
        .unwrap();
    assert!(!first.is_empty());
    let second = crate::radio::radio_rows(&dbs, "album", "ral1")
        .await
        .unwrap();
    assert_eq!(
        first.iter().map(|song| song.id.clone()).collect::<Vec<_>>(),
        second.iter().map(|song| song.id.clone()).collect::<Vec<_>>(),
        "the cached page order is reused"
    );
    crate::radio::clear_cache();
}

/// Online pages use the same local expansion (no server call) and still keep
/// the seed's identity and row count bounded.
#[tokio::test]
async fn online_page_uses_the_local_expansion() {
    let dbs = mini_dbs().await;
    crate::radio::clear_cache();
    let (header, rows) = crate::radio::page(&dbs, "album", "ral1", false)
        .await
        .unwrap();
    assert_eq!(header.title, "Kid A");
    assert_eq!(header.subtitle, "Radio · Radiohead");
    assert_eq!(header.meta, "Similar songs · local");
    assert!(!rows.is_empty());
    crate::radio::clear_cache();
}

/// The queue builder applies the same offline filter as the page, so the
/// clicked row index maps to the same queue position. No HTTP is involved:
/// the stream URL is pure construction from the stored credentials.
#[tokio::test]
async fn offline_queue_matches_the_page_filter() {
    let _offline = lock_offline_catalog().await;
    let dbs = mini_dbs().await;
    for song in ["rs3", "rs4"] {
        sqlx::query(
            "INSERT INTO download (song_id, state, requested_at, updated_at)
             VALUES (?1, 'done', 0, 0)",
        )
        .bind(song)
        .execute(&dbs.download)
        .await
        .unwrap();
    }
    crate::sqlite::save_credentials(
        &dbs.credentials,
        "alice",
        "secret",
        "http://127.0.0.1:9",
        "navidrome",
    )
    .await
    .unwrap();
    crate::offline::clear_cache();
    crate::radio::clear_cache();

    let (tracks, start) = crate::radio::tracks(&dbs, "album", "ral1", 0, true)
        .await
        .unwrap();
    assert!(!tracks.is_empty());
    assert!(start < tracks.len() as i64);
    assert!(
        tracks
            .iter()
            .all(|track| track.url.starts_with("http://127.0.0.1:9/")),
        "offline radio rows resolve without any network call"
    );
    crate::offline::clear_cache();
    crate::radio::clear_cache();
}
