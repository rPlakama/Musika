use super::common::*;
use crate::playlists::*;

/// `create_local`/`add_to_playlist` resolve the process-wide data root. The two
/// folder-writing tests must not race each other's root.
static PLAYLIST_FOLDER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn memory_pool() -> SqlitePool {
    crate::test::memory_pool(None).await
}

fn test_dbs(pools: &SqlitePool, playlist: &SqlitePool) -> Dbs {
    test_dbs_with_credentials(pools, pools, playlist)
}

/// A `Dbs` whose credential pool is distinct from the library pool (the
/// remote-create test saves a server URL there).
fn test_dbs_with_credentials(
    credentials: &SqlitePool,
    library: &SqlitePool,
    playlist: &SqlitePool,
) -> Dbs {
    Dbs {
        credentials: credentials.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers: library.clone(),
        media: library.clone(),
        lyrics: library.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: library.clone(),
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

/// ARC-44: creating from a selection writes one named app-owned M3U, keeps
/// the selection order (and duplicates), overwrites an existing mix of the
/// same name, and leaves it visible in the index immediately.
#[tokio::test]
async fn create_local_writes_a_named_m3u_and_scans_it() {
    let _guard = PLAYLIST_FOLDER_LOCK.lock().await;
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

    // `create_local` uses the active data root; point it at a disposable
    // folder so the test never touches the real playlist directory. No other
    // test resolves the m3u folder, so the process-wide root is safe here.
    let root = std::env::temp_dir().join(format!("musika-create-local-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    crate::db::set_data_root(root.clone());

    let ids = vec!["s2".to_string(), "s1".to_string(), "s2".to_string()];
    let first = create_local(&dbs, "Road Trip", &ids).await.unwrap();
    assert!(first.is_file());
    assert_eq!(first.parent().unwrap(), root.join("playlists"));

    // The same name overwrites the same mix instead of timestamping a copy.
    let second = create_local(&dbs, "Road Trip", &ids).await.unwrap();
    assert!(second.is_file());
    assert_eq!(second, first);
    assert_eq!(second.file_name().unwrap(), "Road Trip.m3u");

    let entries = parse_m3u(&std::fs::read_to_string(&first).unwrap());
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["Music/second.flac", "Music/first.flac", "Music/second.flac"]
    );
    assert_eq!(entries[0].artist.as_deref(), Some("Artist"));
    assert_eq!(entries[0].title.as_deref(), Some("Second"));

    let json = list_json(&dbs).await.unwrap();
    let lists: serde_json::Value = serde_json::from_str(&json).unwrap();
    let lists = lists.as_array().unwrap();
    assert_eq!(lists.len(), 1, "same-name save replaces the mix");
    assert!(lists.iter().any(|list| {
        list["name"] == "Road Trip.m3u" && list["song_count"] == 3 && list["source"] == "m3u"
    }));

    let stored: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT song_id FROM playlist_song WHERE playlist_id = 'm3u:Road Trip.m3u'
         ORDER BY position",
    )
    .fetch_all(&playlist)
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![Some("s2".into()), Some("s1".into()), Some("s2".into())]
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn create_rejects_empty_names_and_empty_song_lists() {
    let library = memory_pool().await;
    let playlist = memory_pool().await;
    let dbs = test_dbs(&library, &playlist);

    assert!(create_local(&dbs, "", &["s1".into()]).await.is_err());
    assert!(create_local(&dbs, "   ", &["s1".into()]).await.is_err());
    assert!(create_local(&dbs, "Mix", &[]).await.is_err());
    assert!(create_local(&dbs, "Mix", &["  ".into()]).await.is_err());

    // Validation happens before the session lookup, so no credentials and no
    // server are needed here.
    assert!(create_remote(&dbs, "", &["s1".into()]).await.is_err());
    assert!(create_remote(&dbs, "Mix", &[]).await.is_err());
}

/// The text helper keeps duplicates/order and decorates only ids the local
/// library knows; unknown (or path-less) ids use the app-owned id convention.
#[tokio::test]
async fn m3u_text_keeps_order_duplicates_and_id_fallbacks() {
    let library = memory_pool().await;
    sqlx::migrate!("./migrations_library")
        .run(&library)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO songs (id, title, artist, duration, path) VALUES
             ('s1', 'First', 'Artist', 100, '/music/first.flac'),
             ('s2', 'Second', NULL, 200, NULL)",
    )
    .execute(&library)
    .await
    .unwrap();
    let playlist = memory_pool().await;
    let dbs = test_dbs(&library, &playlist);

    let ids = ["s1", "missing", "s1", "s2"].map(String::from).to_vec();
    let text = m3u_text(&dbs, &ids).await.unwrap();
    assert!(text.starts_with("#EXTM3U\n"));

    let entries = parse_m3u(&text);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/music/first.flac",
            "musika:song:missing",
            "/music/first.flac",
            "musika:song:s2"
        ]
    );
    assert_eq!(entries[0].artist.as_deref(), Some("Artist"));
    assert_eq!(entries[0].duration, Some(100));
    // Unknown ids stay plain path lines (no fabricated EXTINF).
    assert_eq!(entries[1].duration, None);
    assert_eq!(entries[1].artist, None);
    // A known id without a filesystem path still keeps its cheap metadata.
    assert_eq!(entries[3].title.as_deref(), Some("Second"));
    assert_eq!(entries[3].duration, Some(200));
}

/// The remote half posts the ordered ids, then refreshes `getPlaylists` into
/// the local index — the same lazy-identity model as a normal sync.
#[tokio::test]
async fn create_remote_posts_songs_then_refreshes_the_list() {
    use std::io::{Read, Write};

    let credentials = memory_pool().await;
    sqlx::migrate!("./migrations")
        .run(&credentials)
        .await
        .unwrap();
    // Keep the credential round-trip on the pinned test key instead of the
    // config directory.
    crate::crypto::use_test_key();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let mut requests: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while requests.len() < 2 && Instant::now() < deadline {
            let (mut socket, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(_) => break,
            };
            socket.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut buffer = [0u8; 8192];
            let read = socket.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();

            let body = if request.starts_with("GET /rest/getPlaylists.view") {
                r#"{"subsonic-response":{"status":"ok","version":"1.16.1","playlists":{"playlist":[{"id":"pl1","name":"Mix","songCount":2,"duration":300}]}}}"#
            } else {
                r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(head.as_bytes()).expect("head");
            socket.write_all(body.as_bytes()).expect("body");
            socket.flush().expect("flush");
            requests.push(request);
        }
        requests
    });

    let server_url = format!("http://{addr}");
    crate::sqlite::save_credentials(&credentials, "alice", "secret", &server_url, "navidrome")
        .await
        .unwrap();

    let library = memory_pool().await;
    let playlist = memory_pool().await;
    sqlx::migrate!("./migrations_playlist")
        .run(&playlist)
        .await
        .unwrap();
    let dbs = test_dbs_with_credentials(&credentials, &library, &playlist);

    let ids = vec!["s1".to_string(), "s2".to_string()];
    create_remote(&dbs, "Mix", &ids).await.unwrap();

    let requests = server.join().expect("server thread");
    assert_eq!(requests.len(), 2, "create + list refresh: {requests:?}");
    let create = requests
        .iter()
        .find(|request| request.contains("createPlaylist.view"))
        .expect("createPlaylist request");
    assert!(create.starts_with("GET "));
    assert!(create.contains("name=Mix"));
    let first = create.find("songId=s1").expect("first song");
    let second = create.find("songId=s2").expect("second song");
    assert!(first < second, "song ids keep selection order: {create}");
    assert!(
        requests
            .iter()
            .any(|request| request.contains("getPlaylists.view")),
        "the cached list is refreshed: {requests:?}"
    );

    let name: String = sqlx::query_scalar("SELECT name FROM playlist WHERE id = 'pl1'")
        .fetch_one(&playlist)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT song_count FROM playlist WHERE id = 'pl1'")
        .fetch_one(&playlist)
        .await
        .unwrap();
    assert_eq!(name, "Mix");
    assert_eq!(count, 2);
}

/// `playlist add` resolves free-form targets (artist/song names) and appends
/// their songs to the existing named M3U, preserving target order.
#[tokio::test]
async fn add_to_playlist_appends_resolved_targets() {
    let _guard = PLAYLIST_FOLDER_LOCK.lock().await;
    let library = memory_pool().await;
    sqlx::migrate!("./migrations_library")
        .run(&library)
        .await
        .unwrap();
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Artist', 1);
         INSERT INTO albums (id, artist_id, name, artist, song_count)
             VALUES ('al1', 'ar1', 'Album', 'Artist', 2);
         INSERT INTO songs
             (id, album_id, artist_id, title, artist, album, track, disc_number, duration, path)
         VALUES
             ('s1', 'al1', 'ar1', 'First', 'Artist', 'Album', 1, 1, 100, 'Music/first.flac'),
             ('s2', 'al1', 'ar1', 'Second', 'Artist', 'Album', 2, 1, 200, 'Music/second.flac');",
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
    let root = std::env::temp_dir().join(format!("musika-add-playlist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    crate::db::set_data_root(root.clone());

    let path = create_local(&dbs, "Rated R Mix", &["s1".into()])
        .await
        .unwrap();
    let name = add_to_playlist(&dbs, &["Artist".into(), "Second".into()], "Rated R Mix")
        .await
        .unwrap();
    assert_eq!(name, "Rated R Mix");
    let entries = parse_m3u(&std::fs::read_to_string(&path).unwrap());
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Music/first.flac",
            "Music/first.flac",
            "Music/second.flac",
            "Music/second.flac"
        ]
    );

    let _ = std::fs::remove_dir_all(root);
}
