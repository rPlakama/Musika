use super::common::*;
use crate::playlists::*;

async fn memory_pool() -> SqlitePool {
    crate::test::memory_pool(None).await
}

fn test_dbs(pools: &SqlitePool, playlist: &SqlitePool) -> Dbs {
    Dbs {
        credentials: pools.clone(),
        library: pools.clone(),
        sync: pools.clone(),
        search: pools.clone(),
        covers: pools.clone(),
        media: pools.clone(),
        lyrics: pools.clone(),
        colors: pools.clone(),
        status: pools.clone(),
        artist_user: pools.clone(),
        download: pools.clone(),
        playlist: playlist.clone(),
    }
}

#[tokio::test]
async fn list_skips_empty_playlists() {
    let library = memory_pool().await;
    let playlist = memory_pool().await;
    sqlx::migrate!("./migrations_playlist")
        .run(&playlist)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO playlist (id, source, name, song_count, fetched_at) VALUES
             ('p0', 'subsonic', 'Empty', 0, 0),
             ('p1', 'subsonic', 'Mix', 2, 0)",
    )
    .execute(&playlist)
    .await
    .unwrap();

    let json = list_json(&test_dbs(&library, &playlist)).await.unwrap();
    let rows: serde_json::Value = serde_json::from_str(&json).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "p1");
}

#[tokio::test]
async fn scan_indexes_m3u_files() {
    let library = memory_pool().await;
    sqlx::migrate!("./migrations_library")
        .run(&library)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO songs (id, title, artist, album, duration, path) VALUES
             ('s1', 'ATW', 'All Them Witches', 'ATW', 500, 'Music/a/atw.flac'),
             ('s2', 'Blue Lines', 'Massive Attack', NULL, 300, 'Music/m/blue.flac')",
    )
    .execute(&library)
    .await
    .unwrap();
    let playlist = memory_pool().await;
    sqlx::migrate!("./migrations_playlist")
        .run(&playlist)
        .await
        .unwrap();
    let dbs = test_dbs(&library, &playlist);

    let dir = std::env::temp_dir().join(format!("musika-pl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mix.m3u"),
        "#EXTM3U\n#EXTINF:500,All Them Witches - ATW\nsomewhere/atw.flac\n\
             #EXTINF:300,Massive Attack - Blue Lines\nother/blue.flac\n\
             #EXTINF:1,Ghost - Missing\nghost.flac\n",
    )
    .unwrap();
    let json = scan_dir(&dbs, &dir).await.unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let lists = parsed.as_array().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0]["name"], "mix.m3u");
    assert_eq!(lists[0]["song_count"], 3);

    let rows: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT position, song_id FROM playlist_song ORDER BY position")
            .fetch_all(&playlist)
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
    // Basename fallback, then artist-title fallback, then unmatched kept.
    assert_eq!(rows[0].1.as_deref(), Some("s1"));
    assert_eq!(rows[1].1.as_deref(), Some("s2"));
    assert_eq!(rows[2].1, None);

    let _ = std::fs::remove_dir_all(&dir);
}

/// An empty m3u folder must not read `library.db` at all: the segment is
/// often opened on a machine with no local playlists, and the old scan
/// loaded every song and built three normalized lookup maps first. The
/// library pool here has no schema, so a read would fail the scan.
#[tokio::test]
async fn an_empty_m3u_folder_does_not_read_the_library() {
    let library = memory_pool().await;
    let playlist = memory_pool().await;
    sqlx::migrate!("./migrations_playlist")
        .run(&playlist)
        .await
        .unwrap();
    let dbs = test_dbs(&library, &playlist);

    let dir = std::env::temp_dir().join(format!("musika-pl-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let json = scan_dir(&dbs, &dir).await.expect("empty scan");
    assert_eq!(json, "[]");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parses_extinf_and_paths() {
    let text = "\u{feff}#EXTM3U\r\n# a comment\r\n#EXTINF:214,All Them Witches - ATW\r\nMusic/atw.flac\r\n\r\n#EXTINF:45,Some Title\r\n./rel/song.mp3\r\nplain.ogg\r\n";
    let entries = parse_m3u(text);
    assert_eq!(entries.len(), 3);

    assert_eq!(entries[0].path, "Music/atw.flac");
    assert_eq!(entries[0].artist.as_deref(), Some("All Them Witches"));
    assert_eq!(entries[0].title.as_deref(), Some("ATW"));
    assert_eq!(entries[0].duration, Some(214));

    assert_eq!(entries[1].path, "./rel/song.mp3");
    assert_eq!(entries[1].artist, None);
    assert_eq!(entries[1].title.as_deref(), Some("Some Title"));
    assert_eq!(entries[1].duration, Some(45));

    assert_eq!(entries[2].path, "plain.ogg");
    assert_eq!(entries[2].duration, None);
}

#[test]
fn matcher_falls_back_path_then_base_then_names() {
    let songs = vec![
        SongRef {
            id: "s1".into(),
            title: "ATW".into(),
            artist: Some("All Them Witches".into()),
            album: Some("ATW".into()),
            duration: Some(500),
            cover_art: None,
            path: Some("Music/All Them Witches/atw.flac".into()),
        },
        SongRef {
            id: "s2".into(),
            title: "Blue Lines".into(),
            artist: Some("Massive Attack".into()),
            album: None,
            duration: Some(300),
            cover_art: None,
            path: Some("Music/MA/blue.flac".into()),
        },
    ];
    let matcher = Matcher::new(&songs);
    let dir = Path::new("/playlists");

    // Exact server path.
    let entry = M3uEntry {
        path: "Music/All Them Witches/atw.flac".into(),
        artist: None,
        title: None,
        duration: None,
    };
    assert_eq!(matcher.resolve(&entry, dir), Some(0));

    // Basename fallback (different folder).
    let entry = M3uEntry {
        path: "/home/me/somewhere/blue.flac".into(),
        artist: None,
        title: None,
        duration: None,
    };
    assert_eq!(matcher.resolve(&entry, dir), Some(1));

    // Artist - Title fallback (nothing path-like matches).
    let entry = M3uEntry {
        path: "1-02 ATW.flac".into(),
        artist: Some("All Them Witches".into()),
        title: Some("ATW".into()),
        duration: None,
    };
    assert_eq!(matcher.resolve(&entry, dir), Some(0));

    // App-owned saved-radio entries resolve by song id.
    let saved = M3uEntry {
        path: "musika:song:s2".into(),
        artist: None,
        title: None,
        duration: None,
    };
    assert_eq!(matcher.resolve(&saved, dir), Some(1));

    // Unresolved stays unresolved.
    let entry = M3uEntry {
        path: "missing.flac".into(),
        artist: Some("Nobody".into()),
        title: Some("Nothing".into()),
        duration: None,
    };
    assert_eq!(matcher.resolve(&entry, dir), None);
}

#[tokio::test]
async fn saved_radio_writes_a_local_m3u_and_resolves_ids() {
    let library = memory_pool().await;
    sqlx::migrate!("./migrations_library")
        .run(&library)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO songs (id, title, artist, album, duration, path) VALUES
             ('s1', 'First', 'Artist', 'Album', 100, 'Music/first.flac'),
             ('s2', 'Second', 'Artist', 'Album', 200, 'Music/second.flac')",
    )
    .execute(&library)
    .await
    .unwrap();
    let playlist = memory_pool().await;
    sqlx::migrate!("./migrations_playlist")
        .run(&playlist)
        .await
        .unwrap();
    let dbs = test_dbs(&library, &playlist);
    let rows = vec![
        crate::library::SongRow {
            id: "s1".into(),
            title: "First".into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_id: None,
            track: None,
            duration: Some(100),
            suffix: None,
            cover_art: None,
            starred: None,
        },
        crate::library::SongRow {
            id: "s2".into(),
            title: "Second".into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_id: None,
            track: None,
            duration: Some(200),
            suffix: None,
            cover_art: None,
            starred: None,
        },
        crate::library::SongRow {
            id: "missing".into(),
            title: "Server Only".into(),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            album_id: None,
            track: None,
            duration: Some(300),
            suffix: None,
            cover_art: None,
            starred: None,
        },
    ];
    let dir = std::env::temp_dir().join(format!("musika-radio-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = crate::radio::write_radio_m3u(&dbs, &dir, "Saved Radio", &rows)
        .await
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("musika:song:missing"));
    let ids: Vec<Option<String>> =
        sqlx::query_scalar("SELECT song_id FROM playlist_song ORDER BY position")
            .fetch_all(&playlist)
            .await
            .unwrap();
    assert_eq!(ids, vec![Some("s1".into()), Some("s2".into()), None]);
    let _ = std::fs::remove_dir_all(dir);
}
