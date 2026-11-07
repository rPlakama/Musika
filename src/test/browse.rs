use super::common::*;
use crate::browse::*;

fn song_row(id: &str, album_id: Option<&str>) -> crate::library::SongRow {
    crate::library::SongRow {
        id: id.to_string(),
        title: id.to_string(),
        artist: Some("Radiohead".to_string()),
        album: Some("Kid A".to_string()),
        album_id: album_id.map(str::to_string),
        track: Some(1),
        duration: Some(200),
        suffix: Some("flac".to_string()),
        cover_art: None,
        starred: None,
    }
}

/// ARC-UI-1: the Songs list maps `album_id` into the row so the Alt
/// "Go to Artist" jump can resolve the album's artist.
#[test]
fn song_rows_carry_the_album_id_for_the_alt_jumps() {
    let rows = vec![song_row("s1", Some("al1")), song_row("s2", None)];
    let json = song_rows_json(&rows, &HashMap::new()).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["album_id"], "al1");
    assert!(parsed[1]["album_id"].is_null());
}

/// UI-56: the favourite flag rides the row JSON as a boolean.
#[test]
fn song_rows_carry_the_starred_flag() {
    let mut starred = song_row("s1", Some("al1"));
    starred.starred = Some("2026-10-14T00:00:00Z".to_string());
    let rows = vec![starred, song_row("s2", Some("al1"))];
    let json = song_rows_json(&rows, &HashMap::new()).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed[0]["starred"], true);
    assert_eq!(parsed[1]["starred"], false);
}

/// The streaming serializers must emit exactly the object the old
/// `Value` builders produce: the Qt side parses this shape, so a field
/// rename or a null/absent drift would silently blank a list.
#[test]
fn streaming_list_rows_match_the_value_payload_shape() {
    let covers = HashMap::new();

    let album = crate::library::AlbumRow {
        id: "al1".to_string(),
        title: "Kid A".to_string(),
        artist: Some("Radiohead".to_string()),
        year: Some(2000),
        song_count: 2,
        duration: 0,
        cover_art: None,
        starred: None,
        rating: 3,
    };
    let albums = vec![album];
    let streamed = serde_json::to_string(&AlbumSeq {
        albums: &albums,
        covers: &covers,
        offline: None,
    })
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!([album_json(&albums[0], &covers, None)])
    );

    let artist = crate::library::ArtistRow {
        id: "ar1".to_string(),
        name: "Radiohead".to_string(),
        album_count: 1,
        cover_art: None,
        starred: Some("2026-10-14T00:00:00Z".to_string()),
    };
    let artists = vec![artist];
    let user_starred = HashSet::new();
    let user_ratings = HashMap::new();
    let streamed = serde_json::to_string(&ArtistSeq {
        artists: &artists,
        covers: &covers,
        offline: None,
        user_starred: &user_starred,
        user_ratings: &user_ratings,
    })
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!([artist_json(&artists[0], &covers, None, true, 0)])
    );

    let songs = vec![song_row("s1", Some("al1"))];
    let streamed = song_rows_json(&songs, &covers).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
    let expected = song_json(
        &songs[0].id,
        &songs[0].title,
        songs[0].artist.as_deref(),
        songs[0].album.as_deref(),
        songs[0].album_id.as_deref(),
        songs[0].track,
        songs[0].duration,
        songs[0].suffix.as_deref(),
        crate::covers::url_for(songs[0].cover_art.as_deref(), &covers),
        false,
    );
    assert_eq!(parsed, serde_json::json!([expected]));
    assert_eq!(parsed[0]["suffix"], "flac");

    // The artist page nests the album sequence under `albums`.
    let page = ArtistPage {
        artist: &artists[0],
        cover: "",
        album_count: 1,
        starred: true,
        rating: 0,
        albums: AlbumSeq {
            albums: &albums,
            covers: &covers,
            offline: None,
        },
    };
    let parsed = serde_json::to_value(&page).unwrap();
    assert_eq!(parsed["id"], "ar1");
    assert_eq!(parsed["albums"][0]["id"], "al1");

    // The songs page keeps its paging envelope around the streamed rows.
    let page = SongsPage {
        offset: 10,
        next: 12,
        has_more: true,
        songs: &songs,
        covers: &covers,
    };
    let parsed = serde_json::to_value(&page).unwrap();
    assert_eq!(parsed["offset"], 10);
    assert_eq!(parsed["next"], 12);
    assert_eq!(parsed["has_more"], true);
    assert_eq!(parsed["rows"][0]["id"], "s1");
}

/// The scope path fans a large list across the rayon pool; the joined
/// chunks must equal the sequential payload exactly (same row order, no
/// stray commas), and the join must survive empty chunks.
#[test]
fn parallel_scope_rows_match_the_sequential_payload() {
    assert_eq!(
        join_json_arrays(vec![
            "[{\"id\":1},{\"id\":2}]".to_string(),
            "[{\"id\":3}]".to_string(),
        ]),
        "[{\"id\":1},{\"id\":2},{\"id\":3}]"
    );
    assert_eq!(
        join_json_arrays(vec!["[]".to_string(), "[]".to_string()]),
        "[]"
    );

    let covers = HashMap::new();
    let songs: Vec<_> = (0..1_000)
        .map(|index| song_row(&format!("s{index}"), Some("al1")))
        .collect();
    let parallel = songs_page_json(0, songs.len() as i64, false, &songs, &covers).unwrap();
    let sequential = serde_json::to_string(&SongsPage {
        offset: 0,
        next: songs.len() as i64,
        has_more: false,
        songs: &songs,
        covers: &covers,
    })
    .unwrap();
    assert_eq!(parallel, sequential);

    let albums: Vec<_> = (0..1_000)
        .map(|index| crate::library::AlbumRow {
            id: format!("al{index}"),
            title: format!("Album {index}"),
            artist: Some("Radiohead".to_string()),
            year: Some(2000),
            song_count: 12,
            duration: 0,
            cover_art: None,
            starred: None,
            rating: 0,
        })
        .collect();
    let parallel = album_seq_json(&albums, &covers, None).unwrap();
    let sequential = serde_json::to_string(&AlbumSeq {
        albums: &albums,
        covers: &covers,
        offline: None,
    })
    .unwrap();
    assert_eq!(parallel, sequential);
}

/// Memory probe for the streaming serializers: the same 5k-row album list
/// through the old `Vec<Value>` path and the borrowed `AlbumSeq`.
///
/// `cargo test test::browse::bench_streamed_list_vs_value_tree -- --ignored --nocapture`
#[test]
#[ignore = "memory probe, run explicitly"]
fn bench_streamed_list_vs_value_tree() {
    let albums: Vec<crate::library::AlbumRow> = (0..5_000)
        .map(|index| crate::library::AlbumRow {
            id: format!("al{index}"),
            title: format!("Album number {index}"),
            artist: Some("Radiohead".to_string()),
            year: Some(2000),
            song_count: 12,
            duration: 0,
            cover_art: Some(format!("cov{index}")),
            starred: None,
            rating: 3,
        })
        .collect();
    let covers: HashMap<String, String> = (0..5_000)
        .map(|index| {
            (
                format!("cov{index}"),
                format!("file:///covers/cov{index}.jpg"),
            )
        })
        .collect();

    let (streamed, streamed_peak) = crate::test_alloc::measure(|| {
        serde_json::to_string(&AlbumSeq {
            albums: &albums,
            covers: &covers,
            offline: None,
        })
        .expect("streamed list")
    });
    let (treed, tree_peak) = crate::test_alloc::measure(|| {
        let rows: Vec<serde_json::Value> = albums
            .iter()
            .map(|album| album_json(album, &covers, None))
            .collect();
        serde_json::to_string(&rows).expect("value list")
    });

    eprintln!(
        "album list JSON (5k rows): streamed peak {} KiB | Value-tree peak {} KiB",
        streamed_peak / 1024,
        tree_peak / 1024,
    );
    let streamed: serde_json::Value = serde_json::from_str(&streamed).unwrap();
    let treed: serde_json::Value = serde_json::from_str(&treed).unwrap();
    assert_eq!(
        streamed, treed,
        "the streamed shape must parse to the same JSON value"
    );
    assert!(
        streamed_peak < tree_peak,
        "streaming must not cost more than the intermediate Value tree"
    );
}

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

    let parsed = search_value_index(&dbs, "rdo", false, 20).await.unwrap();
    assert_eq!(parsed["most_played"], false);
    assert_eq!(parsed["album_context"], "al1");
    let ids: Vec<&str> = parsed["songs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|song| song["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["s1", "s2"], "one album match reads in track order");

    let parsed = search_value_index(&dbs, "", false, 20).await.unwrap();
    assert_eq!(parsed["most_played"], true);
    assert!(parsed["album_context"].is_null());
    assert_eq!(parsed["songs"][0]["id"], "s2");
    // The unified search browses every scope on an empty query.
    assert_eq!(parsed["artists"][0]["id"], "ar1");
    assert_eq!(parsed["albums"][0]["id"], "al1");

    crate::search::index::clear();
}

#[test]
fn hybrid_merge_keeps_primary_order_and_dedupes() {
    let mut primary = serde_json::json!([{"id": "a"}, {"id": "b"}]);
    let mut secondary = serde_json::json!([{"id": "b"}, {"id": "c"}, {"id": "d"}]);
    merge_json_rows(&mut primary, &mut secondary, 3);
    let ids: Vec<&str> = primary
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

/// Session-shaped probe: the real list/search loaders against a 20k-song
/// catalog (2k albums, 200 artists), reporting each payload's peak heap
/// and size. This is the Rust half of the "live profile" without Qt.
///
/// `cargo test test::browse::bench_session_loaders -- --ignored --nocapture`
#[test]
#[ignore = "memory probe, run explicitly"]
fn bench_session_loaders() {
    use std::time::Instant;

    fn measured(
        rt: &tokio::runtime::Runtime,
        future: impl std::future::Future<Output = anyhow::Result<String>>,
    ) -> (usize, usize, std::time::Duration) {
        let start = Instant::now();
        let (json, peak) = crate::test_alloc::measure(|| rt.block_on(future).expect("loader"));
        (json.len(), peak.max(0) as usize, start.elapsed())
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("probe runtime");
    let library = rt.block_on(crate::test::memory_pool(sqlx::migrate!(
        "./migrations_library"
    )));
    let covers = rt.block_on(crate::test::memory_pool(sqlx::migrate!(
        "./migrations_media"
    )));
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
    rt.block_on(async {
        sqlx::raw_sql(
            "WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<200)
                 INSERT INTO artists (id, name, album_count, cover_art)
                 SELECT 'ar'||x, 'Artist '||x, 10, 'acov'||x FROM c;
                 WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<2000)
                 INSERT INTO albums
                     (id, artist_id, name, artist, year, song_count, duration, cover_art)
                 SELECT 'al'||x, 'ar'||(((x-1)/10)+1), 'Album '||x,
                        'Artist '||(((x-1)/10)+1), 2000+(x%20), 10, 3600, 'cov'||x
                 FROM c;
                 WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<20000)
                 INSERT INTO songs
                     (id, album_id, artist_id, title, artist, album, track, disc_number,
                      duration, cover_art, play_count)
                 SELECT 's'||x, 'al'||((x-1)/10+1), 'ar'||(((x-1)/100)+1),
                        'Track '||x, 'Artist '||(((x-1)/100)+1),
                        'Album '||((x-1)/10+1), (x%10)+1, 1, 180+(x%120),
                        'cov'||((x-1)/10+1), x%7
                 FROM c;",
        )
        .execute(&library)
        .await
        .expect("seed session");
    });

    let (albums_len, albums_peak, albums_time) =
        measured(&rt, load_albums_json(&dbs, "", "Artist", false));
    eprintln!(
        "session albums (2k): {} KiB payload, {} KiB peak, {:?}",
        albums_len / 1024,
        albums_peak / 1024,
        albums_time
    );

    let (artists_len, artists_peak, artists_time) =
        measured(&rt, load_artists_json(&dbs, "Name", false));
    eprintln!(
        "session artists (200): {} KiB payload, {} KiB peak, {:?}",
        artists_len / 1024,
        artists_peak / 1024,
        artists_time
    );

    let (songs_len, songs_peak, songs_time) =
        measured(&rt, load_songs_page_json(&dbs, 0, 200, "Title", false));
    eprintln!(
        "session songs page (200): {} KiB payload, {} KiB peak, {:?}",
        songs_len / 1024,
        songs_peak / 1024,
        songs_time
    );

    let artist_page = load_artist_json(&dbs, "ar5", "Artist", false);
    let (page_len, page_peak, page_time) = measured(&rt, async {
        artist_page.await.map(|page| page.unwrap_or_default())
    });
    eprintln!(
        "session artist page (10 albums): {} KiB payload, {} KiB peak, {:?}",
        page_len / 1024,
        page_peak / 1024,
        page_time
    );

    // Warm the catalog once, then measure the typed search payload.
    rt.block_on(search_json(&dbs, "artist", false, 30))
        .expect("warm search");
    let (search_len, search_peak, search_time) =
        measured(&rt, search_json(&dbs, "artist", false, 30));
    eprintln!(
        "session search (30 rows/column): {} KiB payload, {} KiB peak, {:?}",
        search_len / 1024,
        search_peak / 1024,
        search_time
    );

    crate::search::index::clear();
}
