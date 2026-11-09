use super::common::*;
use crate::queue::*;

fn song_row(id: &str) -> crate::library::SongRow {
    crate::library::SongRow {
        id: id.to_string(),
        title: id.to_string(),
        artist: Some("Radiohead".to_string()),
        album: Some("Kid A".to_string()),
        album_id: Some("al1".to_string()),
        track: Some(1),
        duration: Some(200),
        suffix: Some("flac".to_string()),
        cover_art: None,
        starred: None,
    }
}

#[test]
fn track_rows_stream_the_player_track_shape() {
    let client =
        crate::http_client::SubsonicClient::new("https://music.example", "alice", "secret");
    let mut song = song_row("s1");
    song.cover_art = Some("cov1".to_string());
    song.starred = Some("2026-10-14T00:00:00Z".to_string());
    let covers = HashMap::from([("cov1".to_string(), "file:///covers/cov1.jpg".to_string())]);
    let media = HashMap::from([("s1".to_string(), PathBuf::from("/media/s1.flac"))]);

    let json = serde_json::to_string(&TrackRows {
        songs: std::slice::from_ref(&song),
        client: &client,
        covers: &covers,
        media: &media,
    })
    .unwrap();
    let tracks: Vec<crate::player::Track> = serde_json::from_str(&json).unwrap();
    assert_eq!(tracks.len(), 1);
    let track = &tracks[0];
    assert_eq!(track.id, "s1");
    assert_eq!(track.artist, "Radiohead");
    assert_eq!(track.album, "Kid A");
    assert_eq!(track.album_id, "al1");
    assert!(track.favorite);
    assert_eq!(track.duration, 200.0);
    assert_eq!(track.cover_art, "cov1");
    assert_eq!(track.cover_url, "file:///covers/cov1.jpg");
    assert_eq!(
        track.url,
        reqwest::Url::from_file_path("/media/s1.flac")
            .unwrap()
            .to_string()
    );

    // No cached blob and no art: the stream URL is used and the
    // cover fields are empty, matching the old `build_track` defaults.
    let plain = song_row("s2");
    let json = serde_json::to_string(&TrackRows {
        songs: std::slice::from_ref(&plain),
        client: &client,
        covers: &HashMap::new(),
        media: &HashMap::new(),
    })
    .unwrap();
    let tracks: Vec<crate::player::Track> = serde_json::from_str(&json).unwrap();
    assert!(tracks[0].url.contains("stream"));
    assert_eq!(tracks[0].cover_art, "");
    assert_eq!(tracks[0].cover_url, "");
    assert!(!tracks[0].favorite);
}

/// Memory probe: a 50k-track queue payload through the old
/// `Vec<Track>` + serialize path vs the borrowing `TrackRows`.
///
/// `cargo test test::queue::bench_track_rows_streaming -- --ignored --nocapture`
#[test]
#[ignore = "memory probe, run explicitly"]
fn bench_track_rows_streaming() {
    let client =
        crate::http_client::SubsonicClient::new("https://music.example", "alice", "secret");
    let songs: Vec<crate::library::SongRow> = (0..50_000)
        .map(|i| {
            let mut song = song_row(&format!("s{i}"));
            song.cover_art = Some(format!("cov{}", i % 2000));
            song.starred = (i % 5 == 0).then(|| "2026-10-14T00:00:00Z".to_string());
            song
        })
        .collect();
    let covers: HashMap<String, String> = (0..2000)
        .map(|i| (format!("cov{i}"), format!("file:///covers/cov{i}.jpg")))
        .collect();
    let media = HashMap::new();

    let (_, vec_peak) = crate::test_alloc::measure(|| {
        let tracks: Vec<crate::player::Track> = songs
            .iter()
            .map(|song| {
                let url = track_url(&client, song, &media).expect("url");
                crate::player::Track {
                    id: song.id.clone(),
                    title: song.title.clone(),
                    artist: song.artist.clone().unwrap_or_default(),
                    album: song.album.clone().unwrap_or_default(),
                    album_id: song.album_id.clone().unwrap_or_default(),
                    favorite: song.starred.is_some(),
                    duration: song.duration.unwrap_or(0) as f64,
                    url,
                    cover_art: song.cover_art.clone().unwrap_or_default(),
                    cover_url: crate::covers::url_for(song.cover_art.as_deref(), &covers),
                    cover_remote: String::new(),
                }
            })
            .collect();
        serde_json::to_string(&tracks).expect("queue json")
    });
    let (_, stream_peak) = crate::test_alloc::measure(|| {
        serde_json::to_string(&TrackRows {
            songs: &songs,
            client: &client,
            covers: &covers,
            media: &media,
        })
        .expect("streamed queue json")
    });

    eprintln!(
        "queue JSON (50k tracks): streamed {} KiB | Vec<Track> {} KiB",
        stream_peak / 1024,
        vec_peak / 1024,
    );
    assert!(
        stream_peak < vec_peak,
        "the streamed queue must not build a Track vector first"
    );
}

#[test]
fn queue_start_selects_the_row_or_zero() {
    let rows = vec![song_row("s1"), song_row("s2")];
    assert_eq!(queue_start(&rows, "s2"), 1);
    // A stale id must still produce a playable queue.
    assert_eq!(queue_start(&rows, "gone"), 0);
}

#[test]
fn library_start_requires_the_clicked_row() {
    let rows = vec![song_row("s1"), song_row("s2"), song_row("s3")];
    assert_eq!(library_start(&rows, "s3").unwrap(), 2);
    // The Songs segment never silently retargets a stale id.
    assert!(library_start(&rows, "gone").is_err());
}

#[test]
fn snapshot_start_survives_dropped_ids() {
    let ids = ["s1".to_string(), "gone".to_string(), "s2".to_string()];
    let rows = vec![song_row("s1"), song_row("s2")];
    // The remembered track survived: resume on it.
    assert_eq!(snapshot_start(&ids, &rows, 2), 1);
    // The remembered track itself is gone: fall back to the head.
    assert_eq!(snapshot_start(&ids, &rows, 1), 0);
    // A truncated snapshot lands on the head too.
    assert_eq!(snapshot_start(&ids, &rows, 9), 0);
}

async fn test_dbs() -> crate::db::Dbs {
    let mut credential_migrations = sqlx::migrate!("./migrations");
    credential_migrations.set_ignore_missing(true);
    let credentials = crate::test::memory_pool(credential_migrations).await;
    let library = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    let covers = crate::test::memory_pool(sqlx::migrate!("./migrations_covers")).await;
    let download = crate::test::memory_pool(sqlx::migrate!("./migrations_download")).await;
    crate::db::Dbs {
        credentials,
        sync: library.clone(),
        library,
        search: crate::test::memory_pool(None).await,
        covers,
        media: crate::test::memory_pool(None).await,
        lyrics: crate::test::memory_pool(None).await,
        colors: crate::test::memory_pool(None).await,
        status: crate::test::memory_pool(None).await,
        artist_user: crate::test::memory_pool(None).await,
        download,
        playlist: crate::test::memory_pool(None).await,
    }
}

#[tokio::test]
async fn offline_filter_keeps_only_downloaded_songs() {
    let dbs = test_dbs().await;
    crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
        .await
        .unwrap();
    crate::download::set_state(&dbs.download, "s2", crate::download::ERROR, Some("x"))
        .await
        .unwrap();

    let mut songs = vec![song_row("s1"), song_row("s2"), song_row("s3")];
    crate::offline::retain_songs(&dbs, true, &mut songs)
        .await
        .unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].id, "s1");

    // Online keeps everything.
    let mut songs = vec![song_row("s1"), song_row("s2")];
    crate::offline::retain_songs(&dbs, false, &mut songs)
        .await
        .unwrap();
    assert_eq!(songs.len(), 2);
}

#[tokio::test]
async fn resume_payload_validates_ids_against_the_index() {
    let dbs = test_dbs().await;
    for (id, artist_id, name, artist, year, songs) in [
        ("al1", "ar1", "Kid A", "Radiohead", 2000, 2),
        ("al2", "ar2", "Absolution", "Muse", 2003, 1),
    ] {
        sqlx::query(
            "INSERT INTO albums (id, artist_id, name, artist, year, song_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(artist_id)
        .bind(name)
        .bind(artist)
        .bind(year)
        .bind(songs)
        .execute(&dbs.library)
        .await
        .unwrap();
    }
    for (id, album_id, title, track) in [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
    {
        sqlx::query(
            "INSERT INTO songs (id, album_id, artist_id, title, artist, album, track, disc_number)
                 VALUES (?1, ?2, 'ar1', ?3, 'Radiohead', 'Kid A', ?4, 1)",
        )
        .bind(id)
        .bind(album_id)
        .bind(title)
        .bind(track)
        .execute(&dbs.library)
        .await
        .unwrap();
    }

    // Known ids: the album page comes back; without credentials the
    // queue degrades to empty instead of failing the restore.
    let status = crate::status::SessionStatus {
        album_id: Some("al1".to_string()),
        song_id: Some("s2".to_string()),
        position: 12.5,
        ..crate::status::SessionStatus::default()
    };
    let payload = resume_payload(&dbs, &status, false).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["album"]["id"], "al1");
    assert_eq!(value["album"]["tracks"], 2);
    assert_eq!(value["queue"], "");
    assert_eq!(value["start"], 0);
    assert_eq!(value["position"], 12.5);

    // Unknown ids behave exactly like a first launch.
    let status = crate::status::SessionStatus {
        album_id: Some("gone".to_string()),
        song_id: Some("gone".to_string()),
        ..crate::status::SessionStatus::default()
    };
    let payload = resume_payload(&dbs, &status, false).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert!(value["album"].is_null());
    assert_eq!(value["queue"], "");
    assert_eq!(value["start"], 0);
    assert_eq!(value["position"], 0.0);
}

#[tokio::test]
async fn quick_actions_resolve_songs_albums_and_pages() {
    let dbs = test_dbs().await;
    for (id, artist_id, name) in [("al1", "ar1", "Kid A"), ("al2", "ar2", "Absolution")] {
        sqlx::query(
            "INSERT INTO albums (id, artist_id, name, artist, year, song_count)
                 VALUES (?1, ?2, ?3, 'Radiohead', 2000, 2)",
        )
        .bind(id)
        .bind(artist_id)
        .bind(name)
        .execute(&dbs.library)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Radiohead', 2)")
        .execute(&dbs.library)
        .await
        .unwrap();
    for (id, album_id, title, track) in [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
    {
        sqlx::query(
            "INSERT INTO songs (id, album_id, artist_id, title, track)
                 VALUES (?1, ?2, 'ar1', ?3, ?4)",
        )
        .bind(id)
        .bind(album_id)
        .bind(title)
        .bind(track)
        .execute(&dbs.library)
        .await
        .unwrap();
    }

    // "Add to Queue" selection: one song, or the album's rows.
    let songs = quick_songs(&dbs, "song", "s2").await.unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].id, "s2");
    let songs = quick_songs(&dbs, "album", "al1").await.unwrap();
    assert_eq!(songs.len(), 2);
    assert!(quick_songs(&dbs, "bogus", "s2").await.is_err());

    // VI-4: a visual selection expands every id, in row order.
    let ids = vec!["s1".to_string(), "s2".to_string()];
    let songs = quick_songs_many(&dbs, "song", &ids).await.unwrap();
    assert_eq!(
        songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["s1", "s2"]
    );
    let ids = vec!["al1".to_string()];
    let songs = quick_songs_many(&dbs, "album", &ids).await.unwrap();
    assert_eq!(songs.len(), 2);

    // ARC-I: an artist hit queues every song of the artist's albums.
    let songs = quick_songs(&dbs, "artist", "ar1").await.unwrap();
    assert_eq!(songs.len(), 2);
    assert!(
        songs
            .iter()
            .all(|song| song.album_id.as_deref() == Some("al1"))
    );
    assert!(
        quick_songs(&dbs, "artist", "gone")
            .await
            .unwrap()
            .is_empty()
    );

    // ARC-I: the search payload album-id map (the search index carries no
    // album ids; the library lookup fills the Go to Artist jump).
    let albums = crate::library::song_album_ids(
        &dbs.library,
        &["s1".to_string(), "s2".to_string(), "gone".to_string()],
    )
    .await
    .unwrap();
    assert_eq!(albums.get("s1").map(String::as_str), Some("al1"));
    assert!(!albums.contains_key("gone"));

    // Navigation payloads carry the shape the pages consume.
    let json = song_album_page_json(&dbs, "s1").await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["id"], "al1");
    assert_eq!(value["tracks"], 2);
    assert!(song_album_page_json(&dbs, "gone").await.is_err());

    let json = album_artist_page_json(&dbs, "al1").await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["id"], "ar1");
    assert_eq!(value["name"], "Radiohead");
    assert!(album_artist_page_json(&dbs, "al2").await.is_err());
}

#[test]
fn shuffle_is_a_permutation_for_any_seed() {
    let source: Vec<String> = (0..64).map(|i| i.to_string()).collect();
    let mut expected = source.clone();
    expected.sort();
    for seed in [
        0_u128,
        1,
        2,
        7,
        42,
        99,
        12_345_678_901_234_567_890,
        u128::MAX,
    ] {
        let mut units = source.clone();
        shuffle_units(&mut units, seed);
        let mut sorted = units.clone();
        sorted.sort();
        assert_eq!(sorted, expected, "every unit survives exactly once");
    }
}

#[test]
fn shuffle_is_deterministic_and_seed_dependent() {
    let source: Vec<String> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .map(String::from)
        .to_vec();
    let mut a = source.clone();
    shuffle_units(&mut a, 1);
    let mut b = source.clone();
    shuffle_units(&mut b, 1);
    assert_eq!(a, b, "same seed, same order");
    let mut c = source.clone();
    shuffle_units(&mut c, 2);
    assert_ne!(a, c, "a different seed walks a different order");
    assert_ne!(a, source, "a shuffle is not the identity (8! odds)");
}

#[test]
fn shuffle_handles_empty_and_singleton() {
    let mut empty: Vec<String> = Vec::new();
    shuffle_units(&mut empty, 7);
    assert!(empty.is_empty());
    let mut one = vec!["only".to_string()];
    shuffle_units(&mut one, 7);
    assert_eq!(one, ["only"]);
}
