use super::common::*;
use crate::library::*;

#[tokio::test]
async fn replace_round_trips_artists_albums_and_songs() {
    let pool = library_pool().await;
    let artist = ArtistID3 {
        id: "ar1".into(),
        name: "Radiohead".into(),
        cover_art: Some("cov1".into()),
        artist_image_url: Some("https://img/ar1".into()),
        album_count: 1,
        starred: Some("2026-01-02T03:04:05Z".into()),
    };
    let album = AlbumID3 {
        id: "al1".into(),
        artist_id: Some("ar1".into()),
        name: "Kid A".into(),
        artist: Some("Radiohead".into()),
        song_count: 1,
        created: Some("2020-10-02T00:00:00Z".into()),
        ..AlbumID3::default()
    };
    let song = Child {
        id: "s1".into(),
        album_id: Some("al1".into()),
        artist_id: Some("ar1".into()),
        title: "Everything in Its Right Place".into(),
        track: Some(1),
        created: Some("2020-10-02T00:00:00Z".into()),
        ..Child::default()
    };

    // Regression: the artists INSERT once listed `starred` twice (7
    // columns for 6 values), failing every sync with SQLite error
    // "6 values for 7 columns".
    replace(&pool, &[artist], &[album], &[song], 42)
        .await
        .unwrap();

    let (name, image, starred): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT name, image_url, starred FROM artists WHERE id = 'ar1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name, "Radiohead");
    assert_eq!(image.as_deref(), Some("https://img/ar1"));
    assert_eq!(starred.as_deref(), Some("2026-01-02T03:04:05Z"));

    let songs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM songs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(songs, 1);
    let (album_created, song_created): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT created FROM albums WHERE id = 'al1'),
                        (SELECT created FROM songs WHERE id = 's1')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(album_created.as_deref(), Some("2020-10-02T00:00:00Z"));
    assert_eq!(song_created.as_deref(), Some("2020-10-02T00:00:00Z"));
    assert_eq!(last_modified(&pool).await.unwrap(), 42);
}

#[tokio::test]
async fn replace_reports_changed_albums_artists_and_covers() {
    let pool = library_pool().await;
    let album = |cover: &str| AlbumID3 {
        id: "al1".into(),
        name: "Kid A".into(),
        cover_art: Some(cover.into()),
        ..AlbumID3::default()
    };
    let artist = |name: &str| ArtistID3 {
        id: "ar1".into(),
        name: name.into(),
        ..ArtistID3::default()
    };

    let first = replace(&pool, &[artist("Radiohead")], &[album("cov-a")], &[], 1)
        .await
        .unwrap();
    assert_eq!(first.albums, vec!["al1".to_string()]);
    assert_eq!(first.artists, vec!["ar1".to_string()]);
    assert_eq!(first.covers, vec!["cov-a".to_string()]);

    let unchanged = replace(&pool, &[artist("Radiohead")], &[album("cov-a")], &[], 2)
        .await
        .unwrap();
    assert!(unchanged.albums.is_empty());
    assert!(unchanged.artists.is_empty());
    assert!(unchanged.covers.is_empty());

    let changed = replace(&pool, &[artist("Radiohead")], &[album("cov-b")], &[], 3)
        .await
        .unwrap();
    assert_eq!(changed.albums, vec!["al1".to_string()]);
    assert_eq!(changed.covers, vec!["cov-b".to_string()]);

    let renamed = replace(&pool, &[artist("Radio Head")], &[album("cov-b")], &[], 4)
        .await
        .unwrap();
    assert!(renamed.albums.is_empty());
    assert_eq!(renamed.artists, vec!["ar1".to_string()]);
    assert!(renamed.covers.is_empty());
}

#[tokio::test]
async fn replace_reports_changed_songs() {
    let pool = library_pool().await;
    let song = |title: &str| Child {
        id: "s1".into(),
        album_id: Some("al1".into()),
        title: title.into(),
        duration: Some(30),
        ..Child::default()
    };

    let first = replace(&pool, &[], &[], &[song("Everything")], 1)
        .await
        .unwrap();
    assert_eq!(first.songs, vec!["s1".to_string()]);

    let unchanged = replace(&pool, &[], &[], &[song("Everything")], 2)
        .await
        .unwrap();
    assert!(unchanged.songs.is_empty());

    let retitled = replace(&pool, &[], &[], &[song("Everything in Its Right Place")], 3)
        .await
        .unwrap();
    assert_eq!(retitled.songs, vec!["s1".to_string()]);
}

#[tokio::test]
async fn album_rating_round_trips_and_syncs_the_user_rating() {
    let pool = library_pool().await;
    let album = AlbumID3 {
        id: "al1".into(),
        name: "Kid A".into(),
        user_rating: Some(4),
        ..AlbumID3::default()
    };
    replace(&pool, &[], &[album], &[], 1).await.unwrap();
    assert_eq!(album_rating(&pool, "al1").await.unwrap(), 4);

    set_album_rating(&pool, "al1", 2).await.unwrap();
    assert_eq!(album_rating(&pool, "al1").await.unwrap(), 2);
    assert_eq!(album_rating(&pool, "missing").await.unwrap(), 0);
}

#[tokio::test]
async fn finds_songs_by_artist_and_title_case_insensitively() {
    let pool = library_pool().await;
    sqlx::query("INSERT INTO songs (id, title, artist) VALUES ('s1', 'Weird Fishes', 'Radiohead')")
        .execute(&pool)
        .await
        .unwrap();

    let by_both = find_song_id(&pool, Some("radiohead"), "WEIRD fishes")
        .await
        .unwrap();
    assert_eq!(by_both.as_deref(), Some("s1"));

    // No artist in the pasted text: title-only match.
    let by_title = find_song_id(&pool, None, "Weird Fishes").await.unwrap();
    assert_eq!(by_title.as_deref(), Some("s1"));

    // Wrong artist must not match.
    assert!(
        find_song_id(&pool, Some("Muse"), "Weird Fishes")
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn order_cycles_both_ways() {
    assert_eq!(cycle_order("Artist", 1), "Name");
    assert_eq!(cycle_order("Name", 1), "Year");
    assert_eq!(cycle_order("Year", 1), "Recent");
    assert_eq!(cycle_order("Recent", 1), "Artist");
    assert_eq!(cycle_order("Artist", -1), "Recent");
    assert_eq!(cycle_order("Recent", -1), "Year");
    assert_eq!(cycle_order("Year", -1), "Name");
    // Unknown values snap back to the first order.
    assert_eq!(cycle_order("", 1), "Name");
}

#[test]
fn artist_order_cycles_both_ways() {
    assert_eq!(cycle_artist_order("Name", 1), "Albums");
    assert_eq!(cycle_artist_order("Albums", 1), "Name");
    assert_eq!(cycle_artist_order("Name", -1), "Albums");
    assert_eq!(cycle_artist_order("", 1), "Albums");
}

#[tokio::test]
async fn artist_orders_switch_between_name_and_album_count() {
    let pool = library_pool().await;
    for (id, name, count) in [
        ("a0", "Empty", 0),
        ("a1", "Zed", 3),
        ("a2", "Alpha", 1),
        ("a3", "Beta", 5),
    ] {
        sqlx::query("INSERT INTO artists (id, name, album_count) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(name)
            .bind(count)
            .execute(&pool)
            .await
            .unwrap();
    }
    // The list is driven by real album rows, not the provider's
    // album_count snapshot.
    for (album, artist) in [("al1", "a1"), ("al2", "a2"), ("al3", "a3")] {
        sqlx::query("INSERT INTO albums (id, artist_id, name) VALUES (?1, ?2, ?3)")
            .bind(album)
            .bind(artist)
            .bind(artist)
            .execute(&pool)
            .await
            .unwrap();
    }
    let by_name: Vec<String> = artists(&pool, "Name")
        .await
        .unwrap()
        .iter()
        .map(|artist| artist.name.clone())
        .collect();
    assert_eq!(by_name, ["Alpha", "Beta", "Zed"]);
    let by_albums: Vec<String> = artists(&pool, "Albums")
        .await
        .unwrap()
        .iter()
        .map(|artist| artist.name.clone())
        .collect();
    assert_eq!(by_albums, ["Beta", "Zed", "Alpha"]);

    let (found, _) = search_artists_albums(&pool, "Empty", 10).await.unwrap();
    assert!(found.is_empty(), "empty artists must not be searchable");
}

#[tokio::test]
async fn artist_albums_follow_the_id_not_the_name() {
    let pool = library_pool().await;
    for (id, name) in [("a1", "Radiohead"), ("a2", "Radiohead"), ("a3", "Muse")] {
        sqlx::query("INSERT INTO artists (id, name) VALUES (?1, ?2)")
            .bind(id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (id, artist_id, name, year) in [
        ("al1", "a1", "Kid A", 2000),
        ("al2", "a1", "OK Computer", 1997),
        ("al3", "a2", "Kid A", 2000),
        ("al4", "a3", "Origin of Symmetry", 2001),
    ] {
        sqlx::query(
            "INSERT INTO albums (id, artist_id, name, artist, year)
                 VALUES (?1, ?2, ?3, 'same-name', ?4)",
        )
        .bind(id)
        .bind(artist_id)
        .bind(name)
        .bind(year)
        .execute(&pool)
        .await
        .unwrap();
    }

    // Same display name, different id: the id decides.
    let ids = |albums: Vec<AlbumRow>| -> Vec<String> {
        albums.into_iter().map(|album| album.id).collect()
    };
    assert_eq!(
        ids(artist_albums(&pool, "a1", "Year").await.unwrap()),
        vec!["al1", "al2"]
    );
    assert_eq!(
        ids(artist_albums(&pool, "a2", "Year").await.unwrap()),
        vec!["al3"]
    );
    assert_eq!(
        ids(artist_albums(&pool, "a1", "Name").await.unwrap()),
        vec!["al1", "al2"]
    );
    assert!(
        artist_albums(&pool, "missing", "Name")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn recent_orders_follow_created_then_name() {
    let pool = library_pool().await;
    for (id, name, created) in [
        ("al-old", "Old", "2020-01-01T00:00:00Z"),
        ("al-new", "New", "2023-01-01T00:00:00Z"),
        ("al-tie", "Alpha", "2023-01-01T00:00:00Z"),
    ] {
        sqlx::query("INSERT INTO albums (id, name, created) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(name)
            .bind(created)
            .execute(&pool)
            .await
            .unwrap();
    }
    let album_ids: Vec<String> = albums(&pool, None, "Recent")
        .await
        .unwrap()
        .into_iter()
        .map(|album| album.id)
        .collect();
    assert_eq!(album_ids, ["al-tie", "al-new", "al-old"]);

    for (id, title, created) in [
        ("s-old", "Old", "2020-01-01T00:00:00Z"),
        ("s-new", "New", "2023-01-01T00:00:00Z"),
        ("s-tie", "Alpha", "2023-01-01T00:00:00Z"),
    ] {
        sqlx::query("INSERT INTO songs (id, title, created) VALUES (?1, ?2, ?3)")
            .bind(id)
            .bind(title)
            .bind(created)
            .execute(&pool)
            .await
            .unwrap();
    }
    let song_ids: Vec<String> = all_songs(&pool, "Recent")
        .await
        .unwrap()
        .into_iter()
        .map(|song| song.id)
        .collect();
    assert_eq!(song_ids, ["s-tie", "s-new", "s-old"]);
}

#[tokio::test]
async fn song_pages_follow_all_songs_order_without_materializing_it() {
    let pool = library_pool().await;
    for (id, artist, album, disc, track, title) in [
        (
            "s1",
            "Radiohead",
            "Kid A",
            1,
            1,
            "Everything In Its Right Place",
        ),
        ("s2", "Radiohead", "Kid A", 1, 2, "Kid A"),
        ("s3", "Muse", "Absolution", 1, 1, "Intro"),
        ("s4", "Muse", "Absolution", 1, 2, "Apocalypse Please"),
        ("s5", "Aurora", "All My Demons", 1, 1, "Runaway"),
    ] {
        sqlx::query(
            "INSERT INTO songs (id, artist, album, disc_number, track, title)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(artist)
        .bind(album)
        .bind(disc)
        .bind(track)
        .bind(title)
        .execute(&pool)
        .await
        .unwrap();
    }

    let all = all_songs(&pool, "Artist").await.unwrap();
    assert_eq!(song_count(&pool).await.unwrap(), all.len());
    for (index, song) in all.iter().enumerate() {
        assert_eq!(
            song_ordinal(&pool, &song.id, "Artist").await.unwrap(),
            Some(index)
        );
    }
    assert_eq!(
        song_ordinal(&pool, "missing", "Artist").await.unwrap(),
        None
    );

    // Pages line up with the same order, whatever the offset.
    let page = songs_range(&pool, 1, 2, "Artist").await.unwrap();
    let ids: Vec<&str> = page.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, vec![all[1].id.as_str(), all[2].id.as_str()]);
    assert!(
        songs_range(&pool, 10, 5, "Artist")
            .await
            .unwrap()
            .is_empty()
    );

    // A different label is the same machinery over another ORDER BY.
    let by_name = songs_range(&pool, 0, 2, "Name").await.unwrap();
    let ids: Vec<&str> = by_name.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, vec!["s4", "s1"]);
    assert_eq!(song_ordinal(&pool, "s4", "Name").await.unwrap(), Some(0));
}

#[tokio::test]
async fn artist_info_round_trips_and_purges_with_the_artist() {
    let pool = library_pool().await;
    sqlx::query("INSERT INTO artists (id, name) VALUES ('a1', 'Pixies')")
        .execute(&pool)
        .await
        .unwrap();

    assert!(artist_info(&pool, "a1").await.unwrap().is_none());

    upsert_artist_info(&pool, "a1", "Boston band.", "https://last.fm/pixies")
        .await
        .unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(row.biography, "Boston band.");
    assert_eq!(row.url, "https://last.fm/pixies");
    assert!(!row.fetched_at.is_empty());

    // A refetch replaces the row, never duplicates it.
    upsert_artist_info(&pool, "a1", "Updated bio.", "")
        .await
        .unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(row.biography, "Updated bio.");
    assert_eq!(row.url, "");

    invalidate_artist_info(&pool, &["a1".to_string()])
        .await
        .unwrap();
    assert!(artist_info(&pool, "a1").await.unwrap().is_none());

    // The artist left the library: the cached biography goes with it.
    upsert_artist_info(&pool, "a1", "Stale bio.", "")
        .await
        .unwrap();
    sqlx::query("DELETE FROM artists WHERE id = 'a1'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(purge_stale_artist_info(&pool).await.unwrap(), 1);
    assert!(artist_info(&pool, "a1").await.unwrap().is_none());
}

#[tokio::test]
async fn similar_artist_graph_round_trips_and_stays_with_the_biography() {
    let pool = library_pool().await;
    sqlx::query("INSERT INTO artists (id, name) VALUES ('a1', 'Pixies')")
        .execute(&pool)
        .await
        .unwrap();

    let similar = vec![
        SimilarArtist {
            id: "a2".to_string(),
            name: "The Breeders".to_string(),
        },
        SimilarArtist {
            id: "a3".to_string(),
            name: "Throwing Muses".to_string(),
        },
    ];
    upsert_artist_similar(&pool, "a1", &similar).await.unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(similar_artists(&row), similar);

    // A biography refetch keeps the graph: separate writers share one row.
    upsert_artist_info(&pool, "a1", "Boston band.", "")
        .await
        .unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(row.biography, "Boston band.");
    assert_eq!(similar_artists(&row), similar);

    // A graph refresh keeps the biography.
    upsert_artist_similar(&pool, "a1", &similar[..1]).await.unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert_eq!(row.biography, "Boston band.");
    assert_eq!(similar_artists(&row), similar[..1].to_vec());

    // Malformed cached JSON degrades to "no graph" instead of failing radio.
    sqlx::query("UPDATE artist_info SET similar = 'not json' WHERE artist_id = 'a1'")
        .execute(&pool)
        .await
        .unwrap();
    let row = artist_info(&pool, "a1").await.unwrap().unwrap();
    assert!(similar_artists(&row).is_empty());

    // Artist changes/removal invalidate the graph with the biography row.
    invalidate_artist_info(&pool, &["a1".to_string()])
        .await
        .unwrap();
    assert!(artist_info(&pool, "a1").await.unwrap().is_none());
}

#[tokio::test]
async fn artist_song_ids_and_file_info_feed_the_downloader() {
    let pool = library_pool().await;
    sqlx::query(
        "INSERT INTO songs (id, title, artist_id, size, suffix, bit_rate)
             VALUES ('s1', 'A', 'a1', 123, 'mp3', 320)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO songs (id, title, artist_id) VALUES ('s2', 'B', 'a1')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO songs (id, title, artist_id) VALUES ('s3', 'C', 'a2')")
        .execute(&pool)
        .await
        .unwrap();

    let mut ids = artist_song_ids(&pool, "a1").await.unwrap();
    ids.sort();
    assert_eq!(ids, vec!["s1", "s2"]);

    assert_eq!(
        song_file_info(&pool, "s1").await.unwrap(),
        Some((Some(123), Some("mp3".to_string()), Some(320)))
    );
    assert_eq!(
        song_file_info(&pool, "s2").await.unwrap(),
        Some((None, None, None))
    );
    assert!(song_file_info(&pool, "missing").await.unwrap().is_none());
}

#[tokio::test]
async fn downloaded_owners_count_only_the_selected_ids() {
    let pool = library_pool().await;
    for (id, album, artist) in [
        ("s1", "al1", "ar1"),
        ("s2", "al1", "ar1"),
        ("s3", "al2", "ar1"),
        ("s4", "al3", "ar2"),
    ] {
        sqlx::query("INSERT INTO songs (id, title, album_id, artist_id) VALUES (?1, ?1, ?2, ?3)")
            .bind(id)
            .bind(album)
            .bind(artist)
            .execute(&pool)
            .await
            .unwrap();
    }

    let ids = vec!["s1".to_string(), "s3".to_string()];
    let (albums, artists) = downloaded_owners(&pool, &ids).await.unwrap();
    assert_eq!(albums.get("al1"), Some(&1));
    assert_eq!(albums.get("al2"), Some(&1));
    assert_eq!(albums.get("al3"), None);
    // ar1 owns two downloaded albums; ar2 owns none.
    assert_eq!(artists.get("ar1"), Some(&2));
    assert_eq!(artists.get("ar2"), None);

    // An empty selection is an empty catalog, not an error.
    let (albums, artists) = downloaded_owners(&pool, &[]).await.unwrap();
    assert!(albums.is_empty());
    assert!(artists.is_empty());
}

/// The album page's format label reads the provider suffix; the shared
/// row select must carry it.
#[tokio::test]
async fn album_songs_carry_the_file_suffix() {
    let pool = library_pool().await;
    sqlx::query(
        "INSERT INTO songs (id, album_id, title, suffix, duration)
             VALUES ('s1', 'al1', 'Track', 'flac', 200)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let songs = album_songs(&pool, "al1").await.unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].suffix.as_deref(), Some("flac"));
}

/// Memory probe: the purge owner check at 100k library songs with a 5k-row
/// cache — full catalog materialization vs `existing_song_ids`.
///
/// `cargo test test::library::bench_existing_song_ids -- --ignored --nocapture`
#[test]
#[ignore = "memory probe, run explicitly"]
fn bench_existing_song_ids() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("probe runtime");
    let pool = rt.block_on(library_pool());
    rt.block_on(async {
        sqlx::raw_sql(
            "INSERT INTO songs (id, title, artist, album, duration, play_count)
                 SELECT printf('s%06d', x), 'Track ' || x, 'Artist', 'Album', 180, 0
                 FROM (WITH RECURSIVE c(x) AS (
                           VALUES(1) UNION ALL SELECT x + 1 FROM c WHERE x < 100000
                       ) SELECT x FROM c);",
        )
        .execute(&pool)
        .await
        .expect("seed songs");
    });

    let derived: Vec<String> = (0..5_000).map(|i| format!("s{:06}", i * 20 + 1)).collect();
    let refs: Vec<&str> = derived.iter().map(String::as_str).collect();

    let (_, full_peak) = crate::test_alloc::measure(|| {
        rt.block_on(async {
            let ids = song_ids(&pool).await.expect("all ids");
            let owners: std::collections::HashSet<String> = ids.into_iter().collect();
            owners.len()
        })
    });
    let (found, derived_peak) = crate::test_alloc::measure(|| {
        rt.block_on(existing_song_ids(&pool, &refs))
            .expect("existing ids")
    });

    eprintln!(
        "owner-set lookup (100k songs, 5k cache): derived-only {} KiB | full catalog {} KiB",
        derived_peak / 1024,
        full_peak / 1024,
    );
    assert_eq!(found.len(), 5_000);
    assert!(
        derived_peak < full_peak,
        "the derived-only lookup must not load the whole catalog"
    );
}

#[tokio::test]
async fn random_album_id_skips_empty_albums() {
    let pool = library_pool().await;
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, album_count) VALUES ('ar1', 'Artist', 2);
         INSERT INTO albums (id, artist_id, name, artist, song_count) VALUES
             ('empty', 'ar1', 'Empty', 'Artist', 0),
             ('full', 'ar1', 'Full', 'Artist', 3);",
    )
    .execute(&pool)
    .await
    .unwrap();
    for _ in 0..8 {
        assert_eq!(random_album_id(&pool).await.unwrap().as_deref(), Some("full"));
    }

    sqlx::query("DELETE FROM albums")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(random_album_id(&pool).await.unwrap(), None);
}
