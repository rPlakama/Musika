use super::common::*;
use super::common::isolated_dbs as test_dbs;
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
fn typed_tracks_carry_the_player_track_shape() {
    let client =
        crate::http_client::SubsonicClient::new("https://music.example", "alice", "secret");
    let mut song = song_row("s1");
    song.cover_art = Some("cov1".to_string());
    song.starred = Some("2026-10-14T00:00:00Z".to_string());
    let covers = HashMap::from([("cov1".to_string(), "file:///covers/cov1.jpg".to_string())]);
    let media = HashMap::from([("s1".to_string(), PathBuf::from("/media/s1.flac"))]);

    let tracks = build_tracks(&client, std::slice::from_ref(&song), &covers, &media).unwrap();
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
    assert!(track.cover_remote.contains("getCoverArt"));
    assert_eq!(
        track.url,
        reqwest::Url::from_file_path("/media/s1.flac")
            .unwrap()
            .to_string()
    );

    // No cached blob and no art: the stream URL is used and the cover fields
    // are empty.
    let plain = song_row("s2");
    let tracks = build_tracks(
        &client,
        std::slice::from_ref(&plain),
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert!(tracks[0].url.contains("stream"));
    assert_eq!(tracks[0].cover_art, "");
    assert_eq!(tracks[0].cover_url, "");
    assert_eq!(tracks[0].cover_remote, "");
    assert!(!tracks[0].favorite);
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

#[tokio::test]
async fn offline_filter_keeps_only_downloaded_songs() {
    let _offline = lock_offline_catalog().await;
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
async fn resume_validates_ids_against_the_index() {
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

    // Known ids: without credentials the queue degrades to empty instead of
    // failing the restore; the saved playhead still comes back.
    let status = crate::status::SessionStatus {
        album_id: Some("al1".to_string()),
        song_id: Some("s2".to_string()),
        position: 12.5,
        ..crate::status::SessionStatus::default()
    };
    let restored = resume(&dbs, &status, false).await.unwrap();
    assert!(restored.tracks.is_empty());
    assert_eq!(restored.start, 0);
    assert_eq!(restored.position, 12.5);

    // Unknown ids behave exactly like a first launch.
    let status = crate::status::SessionStatus {
        album_id: Some("gone".to_string()),
        song_id: Some("gone".to_string()),
        ..crate::status::SessionStatus::default()
    };
    let first_launch = resume(&dbs, &status, false).await.unwrap();
    assert!(first_launch.tracks.is_empty());
    assert_eq!(first_launch.start, 0);
    assert_eq!(first_launch.position, 0.0);
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
