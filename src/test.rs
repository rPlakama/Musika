// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Every unit test, gathered in one place.
//!
//! The production modules are left free of `#[cfg(test)] mod tests` blocks;
//! each nested module below imports its own module and keeps that module's
//! original test helpers and cases.

#![allow(unused_imports)]

mod common {
    pub use crate::api::{AlbumID3, ArtistID3, Child, LyricsList};
    pub use crate::consts::now_secs;
    pub use crate::db::Dbs;
    pub(crate) use crate::digest::md5_hex;
    pub use crate::library::{AlbumRow, ArtistRow, SongSearchRow};
    pub use material_colors::color::Rgb;
    pub use material_colors::dynamic_color::{DynamicScheme, Platform, SpecVersion, Variant};
    pub use material_colors::hct::Hct;
    pub use openssl::hash::MessageDigest;
    pub use serde_json::{Value, json};
    pub use sqlx::SqlitePool;
    pub use std::collections::{HashMap, HashSet};
    pub use std::os::unix::net::UnixStream;
    pub use std::path::{Path, PathBuf};
    pub use std::process::Command;
    pub use std::sync::Arc;
    pub use std::time::{Duration, Instant};
    pub use tokio::sync::mpsc;
}

use sqlx::sqlite::SqlitePool;

async fn memory_pool(migrations: impl Into<Option<sqlx::migrate::Migrator>>) -> SqlitePool {
    use std::str::FromStr;

    let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    if let Some(migrations) = migrations.into() {
        migrations.run(&pool).await.unwrap();
    }
    pool
}

mod api {
    use super::common::*;
    use crate::api::*;

    #[test]
    fn parses_single_object_into_list() {
        let body: Value = serde_json::from_str(
            r#"{"indexes":{"ignoredArticles":"The","lastModified":1,"index":{"name":"A","artist":{"id":"1","name":"AC/DC"}}}}"#,
        )
        .unwrap();

        let indexes: Indexes = extract(body, "indexes").unwrap();
        assert_eq!(indexes.index.len(), 1);
        assert_eq!(indexes.index[0].artist.len(), 1);
        assert_eq!(indexes.index[0].artist[0].name, "AC/DC");
    }

    #[test]
    fn parses_song_type_field_and_missing_fields() {
        let body: Value = serde_json::from_str(
            r#"{"song":{"id":"42","isDir":false,"title":"Hello","artist":"Adele","type":"music"}}"#,
        )
        .unwrap();

        let song: Child = extract(body, "song").unwrap();
        assert_eq!(song.id, "42");
        assert_eq!(song.title, "Hello");
        assert_eq!(song.media_type.as_deref(), Some("music"));
        assert_eq!(song.duration, None);
    }

    #[test]
    fn absent_element_yields_default() {
        let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
        let lyrics: Lyrics = extract(body, "lyrics").unwrap();
        assert!(lyrics.value.is_empty());
    }

    #[test]
    fn album_with_songs_flattens_album_fields() {
        let body: Value = serde_json::from_str(
            r#"{"album":{"id":"a1","name":"Album","songCount":1,"duration":30,"created":"2020-01-01T00:00:00Z","song":[{"id":"s1","isDir":false,"title":"Track"}]}}"#,
        )
        .unwrap();

        let album: AlbumWithSongsID3 = extract(body, "album").unwrap();
        assert_eq!(album.album.name, "Album");
        assert_eq!(album.song.len(), 1);
        assert_eq!(album.song[0].title, "Track");
    }

    #[test]
    fn parses_playlists_and_entries() {
        // A single playlist object becomes a one-element list; fields map.
        let body: Value = serde_json::from_str(
            r#"{"playlists":{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"owner":"me","public":true,"changed":"2026-01-01T00:00:00Z"}}}"#,
        )
        .unwrap();
        let lists: Playlists = extract(body, "playlists").unwrap();
        assert_eq!(lists.playlist.len(), 1);
        assert_eq!(lists.playlist[0].name, "Mix");
        assert_eq!(lists.playlist[0].song_count, Some(2));
        assert_eq!(lists.playlist[0].public, Some(true));

        // The entries flatten into the playlist and keep their order.
        let body: Value = serde_json::from_str(
            r#"{"playlist":{"id":"p1","name":"Mix","songCount":2,"duration":90,"entry":[{"id":"s1","isDir":false,"title":"A"},{"id":"s2","isDir":false,"title":"B"}]}}"#,
        )
        .unwrap();
        let page: PlaylistWithSongs = extract(body, "playlist").unwrap();
        assert_eq!(page.playlist.name, "Mix");
        assert_eq!(page.entry.len(), 2);
        assert_eq!(page.entry[0].title, "A");

        // An absent element defaults to empty, never errors.
        let body: Value = serde_json::from_str(r#"{"version":"1.16.1"}"#).unwrap();
        let lists: Playlists = extract(body, "playlists").unwrap();
        assert!(lists.playlist.is_empty());
    }

    #[test]
    fn query_skips_none() {
        let params = query(&[("a", Some("1".to_string())), ("b", None)]);
        assert_eq!(params, vec![("a", "1".to_string())]);
    }
}

mod artist_user {
    use super::common::*;
    use crate::artist_user::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_artist_user")).await
    }

    #[tokio::test]
    async fn writes_preserve_the_other_field() {
        let pool = memory_pool().await;
        assert!(get(&pool, "a1").await.unwrap().is_none());

        set_starred(&pool, "a1", true).await.unwrap();
        let row = get(&pool, "a1").await.unwrap().unwrap();
        assert!(row.starred);
        assert_eq!(row.rating, 0);

        set_rating(&pool, "a1", 4).await.unwrap();
        let row = get(&pool, "a1").await.unwrap().unwrap();
        assert!(row.starred, "rating must not clear the favourite");
        assert_eq!(row.rating, 4);

        set_starred(&pool, "a1", false).await.unwrap();
        let row = get(&pool, "a1").await.unwrap().unwrap();
        assert!(!row.starred);
        assert_eq!(row.rating, 4, "unstarring must not clear the rating");
    }

    #[tokio::test]
    async fn star_index_lists_only_favourites() {
        let pool = memory_pool().await;
        set_starred(&pool, "a1", true).await.unwrap();
        set_starred(&pool, "a2", false).await.unwrap();
        set_rating(&pool, "a3", 5).await.unwrap();
        assert_eq!(starred_ids(&pool).await.unwrap(), vec!["a1"]);
    }

    #[tokio::test]
    async fn rating_outside_the_domain_is_refused() {
        let pool = memory_pool().await;
        assert!(set_rating(&pool, "a1", 6).await.is_err());
        assert!(get(&pool, "a1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn ratings_map_and_import_skip_invalid_levels() {
        let pool = memory_pool().await;
        set_rating(&pool, "a1", 3).await.unwrap();
        set_rating(&pool, "a2", 0).await.unwrap();

        set_starred(&pool, "a1", true).await.unwrap();

        let map = ratings(&pool).await.unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("a1"), Some(&3));

        import_ratings(
            &pool,
            &[
                ("a1".to_string(), 5),
                ("a2".to_string(), 2),
                ("a3".to_string(), 9),
            ],
        )
        .await
        .unwrap();
        let row = get(&pool, "a1").await.unwrap().unwrap();
        assert_eq!(row.rating, 5);
        assert!(row.starred, "import must preserve the favourite");
        assert_eq!(get(&pool, "a2").await.unwrap().unwrap().rating, 2);
        assert!(get(&pool, "a3").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn payload_defaults_and_reads() {
        let pool = memory_pool().await;
        let unknown: serde_json::Value = serde_json::from_str(&payload(&pool, "a1").await).unwrap();
        assert_eq!(unknown["starred"], false);
        assert_eq!(unknown["rating"], 0);

        set_starred(&pool, "a1", true).await.unwrap();
        set_rating(&pool, "a1", 3).await.unwrap();
        let known: serde_json::Value = serde_json::from_str(&payload(&pool, "a1").await).unwrap();
        assert_eq!(known["starred"], true);
        assert_eq!(known["rating"], 3);
    }
}

mod browse {
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
}

mod cache {
    use super::common::*;
    use crate::cache::*;
    use sqlx::SqlitePool;

    async fn pool() -> SqlitePool {
        crate::test::memory_pool(None).await
    }

    #[tokio::test]
    async fn removes_only_rows_missing_from_the_owner_set() {
        let derived = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&derived)
            .await
            .unwrap();
        for id in ["keep", "gone", "also-gone"] {
            sqlx::query("INSERT INTO cache (id) VALUES (?1)")
                .bind(id)
                .execute(&derived)
                .await
                .unwrap();
        }

        let owners: HashSet<String> = ["keep".to_string()].into_iter().collect();
        assert_eq!(
            purge_missing(&derived, "cache", "id", &owners)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            purge_missing(&derived, "cache", "id", &owners)
                .await
                .unwrap(),
            0
        );
    }

    /// The owner-pool variant asks only about the derived ids; an empty cache
    /// must not query the owner table at all (it can be absent).
    #[tokio::test]
    async fn purge_missing_against_checks_only_the_derived_ids() {
        let derived = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&derived)
            .await
            .unwrap();
        for id in ["keep", "gone", "also-gone"] {
            sqlx::query("INSERT INTO cache (id) VALUES (?1)")
                .bind(id)
                .execute(&derived)
                .await
                .unwrap();
        }
        let owners = pool().await;
        sqlx::query("CREATE TABLE owners (id TEXT PRIMARY KEY)")
            .execute(&owners)
            .await
            .unwrap();
        sqlx::query("INSERT INTO owners (id) VALUES ('keep')")
            .execute(&owners)
            .await
            .unwrap();

        assert_eq!(
            purge_missing_against(&derived, "cache", "id", &owners, "owners", "id")
                .await
                .unwrap(),
            2
        );

        let empty = pool().await;
        sqlx::query("CREATE TABLE cache (id TEXT PRIMARY KEY)")
            .execute(&empty)
            .await
            .unwrap();
        let no_owner_table = pool().await;
        assert_eq!(
            purge_missing_against(&empty, "cache", "id", &no_owner_table, "owners", "id")
                .await
                .unwrap(),
            0
        );
    }

    /// FIX-10: a replace that cannot rename (the destination is a directory)
    /// leaves no `*.tmp` behind.
    #[test]
    fn a_failed_replace_removes_the_temp_file() {
        let dir = std::env::temp_dir().join(format!("musika-atomic-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("cover.jpg")).expect("blocking dir");
        assert!(write_atomic(&dir, "cover.jpg", b"bytes").is_err());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("temp dir")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be cleaned up");
        let _ = std::fs::remove_dir_all(dir);
    }
}

mod colors {
    use super::common::*;
    use crate::colors::engine;
    use crate::colors::seed;
    use crate::colors::*;
    use image::{Rgba, RgbaImage};
    use material_colors::color::Rgb;
    use sqlx::sqlite::SqlitePool;

    /// Writes a synthetic 32x32 cover and returns its path.
    fn write_cover(name: &str, paint: impl Fn(u32, u32) -> Rgba<u8>) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("musika-cover-{}-{name}.png", std::process::id()));
        let mut image = RgbaImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                image.put_pixel(x, y, paint(x, y));
            }
        }
        image.save(&path).unwrap();
        path
    }

    /// Fixed knobs so variant assertions do not depend on smart selection.
    fn tonal() -> Options {
        Options {
            variant: VariantChoice::TonalSpot,
            ..Options::default()
        }
    }

    #[test]
    fn generated_scheme_exposes_the_full_role_map() {
        let path = write_cover("blue", |_, _| Rgba([58, 95, 205, 255]));
        let scheme = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(scheme.scheme, "tonal_spot");
        assert_ne!(scheme.seed, 0);
        assert!(scheme.roles.contains_key("primary"));
        assert!(scheme.roles.len() >= 30);
        assert_eq!(scheme.role_hash.len(), 32);
        for (role, value) in &scheme.roles {
            assert!(
                value.starts_with('#') && value.len() == 7,
                "role {role} is not #rrggbb: {value}"
            );
        }
    }

    #[test]
    fn identical_covers_generate_identical_schemes() {
        let path = write_cover("twice", |x, y| {
            if (x + y) % 2 == 0 {
                Rgba([200, 60, 90, 255])
            } else {
                Rgba([40, 70, 160, 255])
            }
        });
        let first = generate_from_path(&path, &tonal()).unwrap();
        let second = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(first, second);
    }

    #[test]
    fn grayscale_covers_fall_back_without_error() {
        // No color passes the chroma filter; Score returns its own fallback.
        let path = write_cover("gray", |x, _| {
            let level = (x * 8) as u8;
            Rgba([level, level, level, 255])
        });
        let scheme = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(scheme.roles.contains_key("surface"));
        assert!(scheme.roles.contains_key("on_surface"));
    }

    #[test]
    fn fully_transparent_covers_are_rejected() {
        let path = write_cover("clear", |_, _| Rgba([10, 20, 30, 0]));
        let result = generate_from_path(&path, &tonal());
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err());
    }

    #[test]
    fn colorful_seed_prefers_the_vivid_region() {
        // A muted blue field with a tiny vivid red corner: dominant scoring
        // follows the field, caelestia scoring the saturated patch.
        let path = write_cover("vivid", |x, y| {
            if x < 4 && y < 4 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([110, 120, 135, 255])
            }
        });
        let (dominant, _) = seed::extract(&path, SourceMode::Dominant).unwrap();
        let (colorful, _) = seed::extract(&path, SourceMode::Colorful).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_ne!(dominant, colorful, "colorful must not equal dominant");
    }

    // ------------------------------------------------------------------
    // Memory probe (design evidence for `covers::palette_path`)
    // ------------------------------------------------------------------

    /// Palette decode peaks: 320 px thumbnail vs the 2400 px original, as
    /// measured by the test-only heap counter. Run explicitly:
    ///
    /// `cargo test test::colors::bench_palette_decode_peak -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_palette_decode_peak() {
        use image::{ImageFormat, Rgb, RgbImage};

        let dir = std::env::temp_dir().join(format!("musika-palette-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let raw = dir.join("cover.jpg");
        let thumb = dir.join("cover.thumb.jpg");

        // A gradient, not a flat color, so the encoder cannot collapse it.
        let image = RgbImage::from_fn(2400, 2400, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        image
            .save_with_format(&raw, ImageFormat::Jpeg)
            .expect("raw");

        let small = image::open(&raw)
            .expect("decode raw for thumb")
            .thumbnail(320, 320);
        small
            .save_with_format(&thumb, ImageFormat::Jpeg)
            .expect("thumb");
        drop(small);
        drop(image);

        let ((thumb_seed, _), thumb_peak) = crate::test_alloc::measure(|| {
            seed::extract(&thumb, SourceMode::Dominant).expect("thumb seed")
        });
        let ((raw_seed, _), raw_peak) = crate::test_alloc::measure(|| {
            seed::extract(&raw, SourceMode::Dominant).expect("raw seed")
        });

        eprintln!(
            "palette decode heap peak: 320 px thumb {} KiB | 2400 px original {} KiB \
             (thumb colour {thumb_seed:?}, original colour {raw_seed:?})",
            thumb_peak / 1024,
            raw_peak / 1024,
        );
        assert!(
            thumb_peak * 4 < raw_peak,
            "the thumbnail path must cost a fraction of the original decode"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn seed_schemes_are_deterministic_and_serializable() {
        let seed = Rgb::new(58, 95, 205);
        let first = from_seed(seed).unwrap();
        let second = from_seed(seed).unwrap();
        assert_eq!(first, second);

        let payload = first.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["scheme"], "tonal_spot");
        assert_eq!(parsed["roles"]["primary"].as_str().unwrap().len(), 7);
        assert_eq!(parsed["seed"].as_u64().unwrap() as u32, seed.as_argb_u32());
    }

    #[test]
    fn knob_changes_invalidate_the_cache_tag() {
        let base = Options::default();
        assert_ne!(base.generator(), tonal().generator());

        let dark = Options {
            contrast: -1.0,
            ..Options::default()
        };
        assert_ne!(base.generator(), dark.generator());

        let colorful = Options {
            source: SourceMode::Colorful,
            ..Options::default()
        };
        assert_ne!(base.generator(), colorful.generator());

        let spec = Options {
            spec: Spec::Spec2021,
            ..Options::default()
        };
        assert_ne!(base.generator(), spec.generator());

        let seeded = Options {
            seed_override: Some(0xFF3A5FCD),
            ..Options::default()
        };
        assert_ne!(base.generator(), seeded.generator());
    }

    #[test]
    fn contrast_knob_changes_roles() {
        let seed = Rgb::new(58, 95, 205);
        let standard = scheme::build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Dms).1;
        let contrast = scheme::build(seed, VariantChoice::TonalSpot, 0.0, 1.0, Spec::Dms).1;
        assert_ne!(standard, contrast);
    }

    async fn colors_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_colors")).await
    }

    async fn covers_pool() -> SqlitePool {
        let pool = crate::test::memory_pool(None).await;
        sqlx::query("CREATE TABLE covers (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn cache_hit_survives_removing_the_source_image() {
        let pool = colors_pool().await;
        let path = write_cover("cache", |_, _| Rgba([58, 95, 205, 255]));

        let first = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default())
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default())
            .await
            .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn seed_cache_survives_removing_the_source_image() {
        let pool = colors_pool().await;
        let path = write_cover("seed-cache", |_, _| Rgba([58, 95, 205, 255]));

        let first = seed::resolve(&pool, "hash-seed", SourceMode::Colorful, &path, None)
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = seed::resolve(&pool, "hash-seed", SourceMode::Colorful, &path, None)
            .await
            .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn changed_cover_hash_regenerates_from_the_image() {
        let pool = colors_pool().await;
        let path = write_cover("stale", |_, _| Rgba([200, 60, 90, 255]));

        fetch_into(&pool, "cover-2", &path, "hash-a", &Options::default())
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();

        let stale = fetch_into(&pool, "cover-2", &path, "hash-b", &Options::default()).await;
        assert!(stale.is_err(), "a changed hash must regenerate, not reuse");
    }

    #[tokio::test]
    async fn purge_drops_schemes_for_vanished_covers() {
        let colors = colors_pool().await;
        let covers = covers_pool().await;
        sqlx::query("INSERT INTO covers (id) VALUES ('keep')")
            .execute(&covers)
            .await
            .unwrap();

        let path = write_cover("purge", |_, _| Rgba([58, 95, 205, 255]));
        fetch_into(&colors, "keep", &path, "hash", &Options::default())
            .await
            .unwrap();
        fetch_into(&colors, "gone", &path, "hash", &Options::default())
            .await
            .unwrap();
        let _ = std::fs::remove_file(&path);

        let removed = purge_with(&colors, &covers).await.unwrap();
        assert_eq!(removed, 1);
        assert!(engine::load(&colors, "keep").await.unwrap().is_some());
        assert!(engine::load(&colors, "gone").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalidate_regenerates_from_the_cached_seed() {
        let pool = colors_pool().await;
        let path = write_cover("invalidate", |_, _| Rgba([58, 95, 205, 255]));
        let first = fetch_into(&pool, "cover-3", &path, "hash", &Options::default())
            .await
            .unwrap();

        invalidate_with(&pool, "cover-3").await.unwrap();
        assert!(engine::load(&pool, "cover-3").await.unwrap().is_none());
        std::fs::remove_file(&path).unwrap();

        let again = fetch_into(&pool, "cover-3", &path, "hash", &Options::default())
            .await
            .unwrap();
        assert_eq!(first, again, "the seed cache makes regeneration image-free");
    }
}

mod colors_scheme {
    use super::common::*;
    use crate::colors::scheme::*;

    #[test]
    fn smart_thresholds_follow_colorfulness() {
        assert_eq!(smart_variant(90.0), Variant::Vibrant);
        assert_eq!(smart_variant(40.0), Variant::TonalSpot);
        assert_eq!(smart_variant(10.0), Variant::Neutral);
        assert_eq!(smart_variant(2.0), Variant::Monochrome);
    }

    #[test]
    fn specs_produce_different_roles() {
        let seed = Rgb::new(58, 95, 205);
        let (_, roles_2021) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Spec2021);
        let (_, roles_2025) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Spec2025);
        let (_, roles_dms) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Dms);
        assert_ne!(roles_2021, roles_2025, "2021 and 2025 must differ");
        assert_ne!(roles_2025, roles_dms, "DMS raises the accent chroma");
    }

    #[test]
    fn dms_scheme_keeps_neutral_surfaces_from_tonal_spot() {
        let seed = Rgb::new(200, 60, 90);
        let dms = dynamic_scheme(seed, Variant::Vibrant, Spec::Dms, 0.0, true);
        let hct: Hct = seed.into();
        let neutral = DynamicScheme::from_spec(
            hct,
            Variant::TonalSpot,
            true,
            Some(0.0),
            Platform::Phone,
            SpecVersion::Spec2025,
        );
        assert_eq!(dms.neutral_palette.hue(), neutral.neutral_palette.hue());
        assert_eq!(
            dms.neutral_palette.chroma(),
            neutral.neutral_palette.chroma()
        );
    }

    #[test]
    fn all_roles_are_hex() {
        let (name, roles) = build(
            Rgb::new(58, 95, 205),
            VariantChoice::TonalSpot,
            0.0,
            0.0,
            Spec::Dms,
        );
        assert_eq!(name, "tonal_spot");
        assert!(roles.len() >= 30);
        for value in roles.values() {
            assert!(value.starts_with('#') && value.len() == 7, "{value}");
        }
    }
}

mod covers {
    use super::common::*;
    use crate::covers::*;

    /// FIX-2: a claimed id blocks its duplicates until the owning task
    /// releases it (test keys use a unique prefix; the registries are
    /// process-global).
    #[test]
    fn a_claimed_id_blocks_until_released() {
        let fetch_id = format!("claim-test-{}", std::process::id());
        let thumb_name = format!("claim-test-thumb-{}.jpg", std::process::id());
        assert!(claim_fetch(&fetch_id));
        assert!(!claim_fetch(&fetch_id), "duplicate claim is skipped");
        release_fetch(fetch_id.clone());
        assert!(claim_fetch(&fetch_id), "released ids claim again");
        release_fetch(fetch_id);

        assert!(claim_thumb(&thumb_name));
        assert!(!claim_thumb(&thumb_name));
        release_thumb(thumb_name.clone());
        assert!(claim_thumb(&thumb_name));
        release_thumb(thumb_name);
    }

    #[test]
    fn extension_from_content_type() {
        assert_eq!(extension(Some("image/jpeg")), "jpg");
        assert_eq!(extension(Some("image/png; charset=binary")), "png");
        assert_eq!(extension(Some("image/webp")), "webp");
        assert_eq!(extension(None), "img");
        assert_eq!(extension(Some("application/octet-stream")), "img");
    }

    #[test]
    fn md5_is_stable_and_distinct() {
        assert_eq!(md5_hex(b"same").unwrap(), md5_hex(b"same").unwrap());
        assert_ne!(md5_hex(b"a").unwrap(), md5_hex(b"b").unwrap());
    }

    #[test]
    fn raw_file_name_ends_the_upgrade_task() {
        let file = cover_file_name("cov", "hash", "jpg").unwrap();
        assert!(is_raw_file(&file));
        assert!(file.ends_with(".jpg"));
        // A second pass resolves the same canonical name, so the legacy
        // upgrade no longer forces an unconditional fetch.
        assert_eq!(file, cover_file_name("cov", "hash", "jpg").unwrap());
        assert!(!is_raw_file("legacy-cover.jpg"));
    }

    #[test]
    fn svg_detection_by_content_type_and_sniff() {
        assert!(is_svg(b"whatever", Some("image/svg+xml; charset=utf-8")));
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", None));
        assert!(is_svg(b"\n\t<svg/>", None));
        assert!(is_svg(b"\xef\xbb\xbf<svg/>", None));
        assert!(is_svg(
            b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            None
        ));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n", Some("image/png")));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><root/>", None));
    }

    #[test]
    fn svg_is_rasterized_to_a_bounded_png() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2048" height="2048"><rect width="2048" height="2048" fill="red"/></svg>"#;
        let png = rasterize_svg(svg).expect("a valid SVG rasterizes");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let img = image::load_from_memory(&png).expect("stored bytes decode as an image");
        assert_eq!((img.width(), img.height()), (SVG_MAX_EDGE, SVG_MAX_EDGE));
        assert_eq!(img.to_rgba8().get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn small_svg_keeps_its_size() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="lime"/></svg>"#;
        let png = rasterize_svg(svg).expect("a valid SVG rasterizes");
        let img = image::load_from_memory(&png).expect("stored bytes decode as an image");
        assert_eq!((img.width(), img.height()), (64, 32));
    }

    #[test]
    fn malformed_and_oversize_svg_are_rejected() {
        assert!(store_art(b"<svg".to_vec(), Some("image/svg+xml")).is_none());
        let oversize = vec![b' '; SVG_MAX_INPUT + 1];
        assert!(store_art(oversize, Some("image/svg+xml")).is_none());
    }

    #[test]
    fn raster_decoded_art_passes_through() {
        let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
        let (bytes, extension, content_type) =
            store_art(png.clone(), Some("image/png")).expect("raster art passes through");
        assert_eq!(bytes, png);
        assert_eq!(extension, "png");
        assert_eq!(content_type.as_deref(), Some("image/png"));
    }

    fn temp_thumb_dir() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("musika-thumb-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn thumb_name_replaces_only_the_extension() {
        assert_eq!(thumb_name("abc123.jpg"), "abc123.thumb.jpg");
        assert_eq!(thumb_name("abc123"), "abc123.thumb.jpg");
        assert_eq!(thumb_name("a.b.webp"), "a.b.thumb.jpg");
    }

    /// The single-result storage step (the chunked fetch's landing pad): a
    /// new blob is written content-addressed, recorded and queued for its
    /// thumbnail; a `304` only touches the row.
    #[tokio::test]
    async fn apply_fetched_stores_new_art_and_handles_revalidation() {
        use crate::http_client::CoverArt;

        let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_media")).await;
        let dir = temp_thumb_dir();
        let known = HashMap::new();
        let mut report = CoverReport::default();
        let mut jobs = Vec::new();

        let art = CoverArt {
            not_modified: false,
            etag: Some("\"v1\"".to_string()),
            last_modified: None,
            content_type: Some("image/png".to_string()),
            bytes: b"\x89PNG\r\n\x1a\nfake".to_vec(),
        };
        apply_fetched(&pool, &dir, &known, "cov1", Ok(art), &mut report, &mut jobs)
            .await
            .expect("store cover");

        assert_eq!(report.downloaded, 1);
        assert_eq!(jobs.len(), 1);
        let (file, hash, size): (String, String, i64) =
            sqlx::query_as("SELECT file, hash, size FROM covers WHERE id = 'cov1'")
                .fetch_one(&pool)
                .await
                .expect("cover row");
        assert!(file.ends_with(".png"), "content type picks the extension");
        assert_eq!(hash.len(), 32);
        assert_eq!(size, 12);
        assert!(dir.join(&file).is_file());
        assert_eq!(jobs[0].1, dir.join(&file));

        let revalidated = CoverArt {
            not_modified: true,
            etag: None,
            last_modified: None,
            content_type: None,
            bytes: Vec::new(),
        };
        apply_fetched(
            &pool,
            &dir,
            &known,
            "cov1",
            Ok(revalidated),
            &mut report,
            &mut jobs,
        )
        .await
        .expect("touch cover");
        assert_eq!(report.unchanged, 1);
        assert_eq!(jobs.len(), 1, "a 304 never queues a thumbnail");

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The palette decodes the 320 px thumbnail when it exists (the whole
    /// point of the thumbnail) and falls back to the original while it does
    /// not (a just-fetched cover, offline).
    #[test]
    fn palette_art_prefers_the_thumbnail_then_the_original() {
        let dir = temp_thumb_dir();
        let raw = dir.join("raw-abc.jpg");
        std::fs::write(&raw, b"raw").expect("write raw");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(raw.as_path()),
            "no thumbnail yet: the full cover is the only decodable art"
        );

        let thumb = dir.join("raw-abc.thumb.jpg");
        std::fs::write(&thumb, b"thumb").expect("write thumb");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(thumb.as_path()),
            "a present thumbnail must win"
        );

        std::fs::remove_file(&raw).expect("remove raw");
        assert_eq!(
            preferred_art(&dir, "raw-abc.jpg").as_deref(),
            Some(thumb.as_path()),
            "a missing original must not hide an existing thumbnail"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumb_is_bounded_and_written() {
        let dir = temp_thumb_dir();
        let source = dir.join("cover.png");
        image::RgbaImage::from_pixel(640, 480, image::Rgba([10, 20, 30, 255]))
            .save(&source)
            .expect("write cover");

        let bytes = build_thumb(&source).expect("build thumbnail");
        let thumb = image::load_from_memory(&bytes).expect("thumbnail decodes");
        assert_eq!((thumb.width(), thumb.height()), (THUMB_EDGE, 240));

        let name = thumb_name("cover.png");
        write_thumb(&source, &dir, &name).expect("write thumbnail");
        assert!(dir.join(&name).is_file());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumb_falls_back_to_the_original_blob() {
        let dir = temp_thumb_dir();
        let source = dir.join("broken.img");
        std::fs::write(&source, b"not an image").expect("write blob");

        let name = thumb_name("broken.img");
        write_thumb(&source, &dir, &name).expect("fallback copy");
        assert_eq!(
            std::fs::read(dir.join(&name)).expect("read thumb"),
            b"not an image"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The size split still processes every job: small art decodes in the
    /// parallel path and an oversized blob goes through the single slot
    /// (falling back to a copy when it is not decodable).
    #[tokio::test]
    async fn ensure_thumbs_covers_both_size_buckets() {
        use image::{Rgba, RgbaImage};

        let dir = temp_thumb_dir();
        let small = dir.join("small.png");
        RgbaImage::from_pixel(64, 64, Rgba([10, 20, 30, 255]))
            .save(&small)
            .expect("small cover");
        let large = dir.join("large.img");
        std::fs::write(&large, vec![0u8; THUMB_LARGE_BYTES as usize + 1]).expect("large blob");

        let processed = ensure_thumbs(
            &dir,
            vec![
                (thumb_name("small.png"), small),
                (thumb_name("large.img"), large),
            ],
        )
        .await;

        assert_eq!(processed.len(), 2);
        assert!(dir.join("small.thumb.jpg").is_file());
        assert!(dir.join("large.thumb.jpg").is_file());
        assert_eq!(
            std::fs::read(dir.join("large.thumb.jpg"))
                .expect("fallback thumb")
                .len(),
            THUMB_LARGE_BYTES as usize + 1
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

mod crypto {
    use super::common::*;
    use crate::crypto::*;

    fn test_key() -> SecretKey {
        SecretKey::from_bytes([7u8; KEY_LEN])
    }

    #[test]
    fn round_trip() {
        let key = test_key();
        let encoded = key.encrypt("hunter2").unwrap();

        assert!(crate::hex::decode(&encoded).is_ok());
        assert_eq!(key.decrypt(&encoded).unwrap(), "hunter2");
    }

    #[test]
    fn wrong_key_fails() {
        let encoded = test_key().encrypt("hunter2").unwrap();
        let other = SecretKey::from_bytes([9u8; KEY_LEN]);

        assert!(other.decrypt(&encoded).is_err());
    }

    #[test]
    fn encryption_is_randomised() {
        let key = test_key();

        assert_ne!(key.encrypt("same").unwrap(), key.encrypt("same").unwrap());
    }
}

mod db {
    use super::common::*;
    use crate::db::*;

    #[test]
    fn library_migrations_include_the_combined_schema() {
        let migrations = sqlx::migrate!("./migrations_library");
        assert!(
            migrations
                .iter()
                .any(|migration| migration.version == 20261017000001)
        );
    }

    #[tokio::test]
    async fn wipe_pool_drops_every_table_and_migrations_rebuild_them() {
        let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
        sqlx::query("INSERT INTO artists (id, name) VALUES ('a', 'A')")
            .execute(&pool)
            .await
            .unwrap();

        wipe_pool(&pool).await.unwrap();
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 0);

        // Re-migrating yields the empty schema again (the wipe path's second
        // half): tables exist, rows are gone.
        sqlx::migrate!("./migrations_library")
            .run(&pool)
            .await
            .unwrap();
        let artists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artists")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(artists, 0);
    }
}

mod download {
    use super::common::*;
    use crate::download::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_download")).await
    }

    #[tokio::test]
    async fn runs_the_state_machine_and_never_downgrades_done() {
        let pool = memory_pool().await;
        assert!(get(&pool, "s1").await.unwrap().is_none());

        mark_queued(&pool, "s1", Some("r1")).await.unwrap();
        let row = get(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(row.state, QUEUED);
        assert_eq!(row.rule_id.as_deref(), Some("r1"));

        set_state(&pool, "s1", ACTIVE, None).await.unwrap();
        set_state(&pool, "s1", ERROR, Some("network down"))
            .await
            .unwrap();
        let row = get(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(row.state, ERROR);
        assert_eq!(row.error.as_deref(), Some("network down"));

        // Re-selecting a failed song retries it (clears the error)...
        mark_queued(&pool, "s1", None).await.unwrap();
        let row = get(&pool, "s1").await.unwrap().unwrap();
        assert_eq!(row.state, QUEUED);
        assert_eq!(row.error, None);

        // ...but a finished row stays finished.
        set_state(&pool, "s1", DONE, None).await.unwrap();
        mark_queued(&pool, "s1", None).await.unwrap();
        assert_eq!(get(&pool, "s1").await.unwrap().unwrap().state, DONE);
    }

    #[tokio::test]
    async fn queued_ids_are_oldest_first_and_bounded() {
        let pool = memory_pool().await;
        for id in ["s2", "s1", "s3"] {
            mark_queued(&pool, id, None).await.unwrap();
        }
        set_state(&pool, "s3", ACTIVE, None).await.unwrap();
        assert_eq!(queued_ids(&pool, 10).await.unwrap().len(), 2);
        assert_eq!(queued_ids(&pool, 1).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn done_ids_lists_only_finished_songs() {
        let pool = memory_pool().await;
        mark_queued(&pool, "s1", None).await.unwrap();
        set_state(&pool, "s2", DONE, None).await.unwrap();
        set_state(&pool, "s3", ERROR, Some("x")).await.unwrap();
        set_state(&pool, "s1", DONE, None).await.unwrap();

        let mut ids = done_ids(&pool).await.unwrap();
        ids.sort();
        assert_eq!(ids, ["s1", "s2"]);
    }

    #[tokio::test]
    async fn counts_and_unmark() {
        let pool = memory_pool().await;
        mark_queued(&pool, "s1", Some("r1")).await.unwrap();
        mark_queued(&pool, "s2", Some("r1")).await.unwrap();
        set_state(&pool, "s2", DONE, None).await.unwrap();
        set_state(&pool, "s3", ERROR, Some("x")).await.unwrap();

        let snapshot = counts(&pool).await.unwrap();
        assert_eq!(
            snapshot,
            Counts {
                queued: 1,
                active: 0,
                done: 1,
                error: 1,
                total: 3
            }
        );

        // The rule's songs are readable before the marks go: the removal
        // path uses them to catch blobs stored without the rule id.
        let mut rule_songs = rule_song_ids(&pool, "r1").await.unwrap();
        rule_songs.sort();
        assert_eq!(rule_songs, ["s1", "s2"]);
        assert!(rule_song_ids(&pool, "nope").await.unwrap().is_empty());

        // Rule removal drops its pending *and* finished rows (the blobs go
        // with the rule); explicit downloads have no rule id and survive.
        assert_eq!(unmark_rule(&pool, "r1").await.unwrap(), 2);
        assert_eq!(counts(&pool).await.unwrap().total, 1);

        unmark(&pool, "s3").await.unwrap();
        assert_eq!(counts(&pool).await.unwrap().total, 0);
    }

    #[tokio::test]
    async fn window_sets_split_done_from_in_flight() {
        let pool = memory_pool().await;
        mark_queued(&pool, "a", None).await.unwrap();
        set_state(&pool, "a", ACTIVE, None).await.unwrap();
        mark_queued(&pool, "b", None).await.unwrap();
        set_state(&pool, "b", DONE, None).await.unwrap();
        mark_queued(&pool, "c", None).await.unwrap();
        set_state(&pool, "c", ERROR, Some("nope")).await.unwrap();

        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(done_ids_of(&pool, &ids).await.unwrap(), vec!["b"]);
        assert_eq!(pending_ids_of(&pool, &ids).await.unwrap(), vec!["a"]);
        assert!(pending_ids_of(&pool, &[]).await.unwrap().is_empty());
    }
}

mod fanout {
    use super::common::*;
    use crate::fanout::*;

    #[tokio::test]
    async fn maps_every_id_and_collects_errors() {
        let ids: Vec<String> = (0..20).map(|id| id.to_string()).collect();
        let results = map_bounded(ids.clone(), |id| async move {
            if id == "7" {
                anyhow::bail!("failed");
            }
            Ok(id.parse::<usize>().unwrap())
        })
        .await;

        assert_eq!(results.len(), ids.len());
        let mut ok = Vec::new();
        let mut failed = Vec::new();
        for (id, result) in results {
            match result {
                Ok(value) => ok.push((id, value)),
                Err(_) => failed.push(id),
            }
        }
        assert_eq!(failed, vec!["7"]);
        assert_eq!(ok.len(), 19);
        assert!(ok.contains(&("19".to_string(), 19)));
    }
}

mod hex {
    use super::common::*;
    use crate::hex::*;

    #[test]
    fn round_trips_bytes() {
        let bytes = [0x00, 0x0f, 0xa5, 0xff, b'm', b'd'];
        assert_eq!(decode(&encode(bytes)).unwrap(), bytes);
    }

    #[test]
    fn accepts_uppercase() {
        assert_eq!(decode("DEADBEEF").unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode("abc").is_err());
        assert!(decode("zz").is_err());
    }
}

mod http_client {
    use super::common::*;
    use crate::http_client::*;

    #[test]
    fn endpoint_url_has_auth_and_extra_params() {
        let client = SubsonicClient::new("navidrome.example.com/", "alice", "secret");
        let url = client
            .endpoint_url("getArtists", &[("musicFolderId", "1".to_string())])
            .unwrap();

        assert_eq!(url.scheme(), "https");
        assert_eq!(url.path(), "/rest/getArtists.view");

        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("u").unwrap(), "alice");
        assert_eq!(query.get("v").unwrap(), API_VERSION);
        assert_eq!(query.get("c").unwrap(), CLIENT_NAME);
        assert_eq!(query.get("f").unwrap(), "json");
        assert_eq!(query.get("musicFolderId").unwrap(), "1");
        assert!(query.contains_key("t") && query.contains_key("s"));
    }

    #[test]
    fn token_is_md5_of_password_plus_salt() {
        let expected = openssl::hash::hash(MessageDigest::md5(), b"secretabc").unwrap();
        assert_eq!(
            md5_token("secret", "abc").unwrap(),
            crate::hex::encode(expected)
        );
    }

    #[test]
    fn ok_envelope_yields_body() {
        let envelope: Envelope<serde_json::Value> =
            serde_json::from_str(r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#)
                .unwrap();
        assert!(envelope.response.into_result().is_ok());
    }

    #[test]
    fn failed_envelope_surfaces_server_message() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("Wrong username or password"));
    }

    #[test]
    fn auth_code_without_message_still_maps_to_credentials_error() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":40}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("Wrong username or password"));
    }

    #[test]
    fn other_errors_are_not_masked_as_auth_failures() {
        let envelope: Envelope<serde_json::Value> = serde_json::from_str(
            r#"{"subsonic-response":{"status":"failed","error":{"code":70}}}"#,
        )
        .unwrap();

        let err = envelope.response.into_result().unwrap_err();
        assert!(err.to_string().contains("code 70"));
        assert!(!err.to_string().contains("username"));
    }

    /// The download path streams the body to a file instead of collecting it:
    /// a multi-chunk body must arrive intact and the byte count must match.
    #[tokio::test]
    async fn media_download_streams_chunks_to_the_target_file() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let body: Vec<u8> = (0..200_000).map(|index| (index % 251) as u8).collect();
        let served = body.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                served.len()
            );
            socket.write_all(head.as_bytes()).expect("head");
            for chunk in served.chunks(16 * 1024) {
                socket.write_all(chunk).expect("chunk");
            }
            socket.flush().expect("flush");
        });

        let dir = std::env::temp_dir().join(format!("musika-download-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let dest = dir.join("song.part");

        let client = SubsonicClient::new(&format!("http://{addr}"), "alice", "secret");
        let written = client
            .download_media_to("song-1", &dest)
            .await
            .expect("stream the body");
        server.join().expect("server thread");

        assert_eq!(written as usize, body.len());
        assert_eq!(std::fs::read(&dest).expect("read back"), body);
        let _ = std::fs::remove_dir_all(dir);
    }
}

mod index {
    use super::common::*;
    use crate::index::*;

    #[test]
    fn provider_date_normalises_milliseconds() {
        assert_eq!(provider_secs(1_700_000_000), 1_700_000_000);
        assert_eq!(provider_secs(1_700_000_000_123), 1_700_000_000);
        assert_eq!(provider_secs(0), 0);
    }

    #[test]
    fn complete_fetches_pass_and_partial_ones_fail() {
        assert!(ensure_complete(&[], 10).is_ok());

        let err = ensure_complete(&["al-1".to_string()], 10).unwrap_err();
        assert!(err.to_string().contains("1 of 10"));
        assert!(err.to_string().contains("al-1"));

        let err = ensure_complete(&["a".to_string(), "b".to_string()], 3).unwrap_err();
        assert!(err.to_string().contains("2 of 3"));
    }
}

mod ipc {
    use super::common::*;
    use crate::ipc::*;
    use std::io::Write;
    use std::time::{Duration, Instant};

    fn wait_connect(path: &Path) -> UnixStream {
        let start = Instant::now();
        loop {
            match UnixStream::connect(path) {
                Ok(stream) => return stream,
                Err(err) => {
                    assert!(
                        start.elapsed() < Duration::from_secs(2),
                        "ipc socket never came up: {err}"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// FIX-9: a client that connects and stays silent must not block a
    /// second client's command.
    #[test]
    fn a_silent_client_does_not_block_later_commands() {
        let dir = std::env::temp_dir().join(format!("musika-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("test.sock");

        let (tx, rx) = std::sync::mpsc::channel::<String>();
        serve(path.clone(), move |line| {
            let _ = tx.send(line);
        });

        let silent = wait_connect(&path);
        let mut second = wait_connect(&path);
        second.write_all(b"ping\n").expect("write command");

        let received = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the second client's command ran");
        assert_eq!(received, "ping");

        drop(silent);
        drop(second);
        let _ = std::fs::remove_dir_all(dir);
    }
}

mod keybind {
    use super::common::*;
    use crate::keybind::*;

    #[test]
    fn single_token_binding() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-n"), Outcome::Action(Action::NextTrack));
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
    }

    #[test]
    fn sequences_need_all_tokens() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Action(Action::Top));

        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("f"), Outcome::Action(Action::Search));
    }

    #[test]
    fn dead_end_resets_and_retries() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        // Second token does not continue `g g`; the single `j` still fires.
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert!(!keymap.is_pending());
    }

    #[test]
    fn unknown_key_is_none() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("f13"), Outcome::None);
    }

    #[test]
    fn qt_tokens() {
        assert_eq!(token_from_qt(0x4A, 0).as_deref(), Some("j")); // Key_J
        assert_eq!(token_from_qt(0x4E, QT_CTRL).as_deref(), Some("ctrl-n"));
        assert_eq!(token_from_qt(0x57, QT_CTRL).as_deref(), Some("ctrl-w"));
        assert_eq!(token_from_qt(0x77, QT_CTRL).as_deref(), Some("ctrl-w"));
        assert_eq!(token_from_qt(0x47, QT_SHIFT).as_deref(), Some("shift-g"));
        assert_eq!(token_from_qt(QT_KEY_SPACE, 0).as_deref(), Some("space"));
        assert_eq!(token_from_qt(QT_KEY_ESCAPE, 0).as_deref(), Some("escape"));
        assert_eq!(token_from_qt(QT_KEY_DOWN, 0).as_deref(), Some("down"));
        assert_eq!(
            token_from_qt(QT_KEY_ENTER, QT_SHIFT).as_deref(),
            Some("shift-enter")
        );
        assert_eq!(token_from_qt(0x0100_0009, 0), None); // unbound special key
        assert_eq!(token_from_qt(0x2F, 0).as_deref(), Some("/")); // Key_Slash
        assert_eq!(token_from_qt(0x5B, 0).as_deref(), Some("[")); // Key_BracketLeft
        assert_eq!(token_from_qt(0x5D, 0).as_deref(), Some("]")); // Key_BracketRight
        assert_eq!(token_from_qt(0x3F, 0), None); // `?` stays unbound until M4
    }

    #[test]
    fn place_history_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("["), Outcome::Pending);
        assert_eq!(keymap.feed("b"), Outcome::Action(Action::PrevPlace));
        assert_eq!(keymap.feed("]"), Outcome::Pending);
        assert_eq!(keymap.feed("b"), Outcome::Action(Action::NextPlace));

        // A lone prefix times out silently; Ctrl-[ keeps hierarchical Back.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("["), Outcome::Pending);
        assert_eq!(keymap.feed_timeout(), Outcome::None);
        assert_eq!(keymap.feed("ctrl-["), Outcome::Action(Action::Back));
    }

    #[test]
    fn slash_opens_the_surface_search() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("/"), Outcome::Action(Action::SearchAction));
    }

    #[test]
    fn shift_enter_toggles_fullscreen() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("shift-enter"),
            Outcome::Action(Action::ToggleFullscreen)
        );
    }

    #[test]
    fn space_plays_and_shift_space_f_searches() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("space"), Outcome::Action(Action::PlayPause));

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("f"), Outcome::Action(Action::Search));

        // Shift held through to F is the natural way to type Shift+Space+F.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-space"), Outcome::Pending);
        assert_eq!(keymap.feed("shift-f"), Outcome::Action(Action::Search));
    }

    #[test]
    fn escape_and_insert_mode() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert_eq!(keymap.feed("i"), Outcome::Action(Action::InsertMode));
    }

    #[test]
    fn ctrl_bracket_is_back_and_cancels_pending() {
        assert_eq!(token_from_qt(0x5B, QT_CTRL).as_deref(), Some("ctrl-["));

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-["), Outcome::Action(Action::Back));

        // A half-typed sequence is cleared by the escape alias.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert!(!keymap.is_pending());
    }

    #[test]
    fn visual_selection_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("v"), Outcome::Action(Action::VisualSelect));
        assert_eq!(keymap.feed("shift-v"), Outcome::Action(Action::VisualAll));
        assert_eq!(token_from_qt(0x56, QT_SHIFT).as_deref(), Some("shift-v"));
        // Ctrl-W is retired: `/` owns cross-region reach (ARC-UI-2), so the
        // token stays a plain no-binding now.
        assert_eq!(keymap.feed("ctrl-w"), Outcome::None);
    }

    #[test]
    fn queue_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-q"), Outcome::Action(Action::QueueToggle));
        assert_eq!(
            keymap.feed("ctrl-shift-j"),
            Outcome::Action(Action::QueueDown)
        );
        assert_eq!(
            keymap.feed("ctrl-shift-k"),
            Outcome::Action(Action::QueueUp)
        );

        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("d"), Outcome::Pending);
        assert_eq!(keymap.feed("d"), Outcome::Action(Action::QueueRemove));
    }

    #[test]
    fn backtab_canonicalises_to_shift_tab() {
        assert_eq!(token_from_qt(0x0100_0002, 0).as_deref(), Some("shift-tab"));
        assert_eq!(token_from_qt(0x0100_0001, 0).as_deref(), Some("tab"));
        assert_eq!(
            token_from_qt(0x0100_0001, QT_SHIFT).as_deref(),
            Some("shift-tab")
        );
    }

    #[test]
    fn transport_and_pane_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("n"),
            Outcome::None,
            "bare n is free (VI-8a/VI-14)"
        );
        assert_eq!(keymap.feed("p"), Outcome::Action(Action::PasteBelow));
        assert_eq!(keymap.feed("ctrl-n"), Outcome::Action(Action::NextTrack));
        assert_eq!(keymap.feed("ctrl-p"), Outcome::Action(Action::PrevTrack));
        assert_eq!(keymap.feed("tab"), Outcome::Action(Action::PaneCycle));
        assert_eq!(
            keymap.feed("shift-tab"),
            Outcome::Action(Action::PaneCycleBack)
        );
    }

    #[test]
    fn seek_keys_are_global_bindings() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed(","), Outcome::Action(Action::SeekBack));
        assert_eq!(keymap.feed("."), Outcome::Action(Action::SeekForward));
        assert_eq!(token_from_qt(0x2C, 0).as_deref(), Some(","));
        assert_eq!(token_from_qt(0x2E, 0).as_deref(), Some("."));
    }

    #[test]
    fn refresh_key_syncs_the_library() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("r"), Outcome::Action(Action::Refresh));
        assert_eq!(token_from_qt(0x52, 0).as_deref(), Some("r"));
    }

    #[test]
    fn clipboard_verbs() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("y"), Outcome::Action(Action::Yank));
        assert_eq!(keymap.feed("shift-y"), Outcome::Action(Action::YankRow));
        assert_eq!(keymap.feed("p"), Outcome::Action(Action::PasteBelow));
        assert_eq!(keymap.feed("shift-p"), Outcome::Action(Action::PasteAbove));
        assert_eq!(keymap.feed("ctrl-v"), Outcome::Action(Action::Paste));
        assert_eq!(
            token_from_qt(0x56, QT_CTRL).as_deref(),
            Some("ctrl-v"),
            "system paste (Ctrl-V) must map to the text clipboard action"
        );
    }

    #[test]
    fn volume_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("m"), Outcome::Action(Action::Mute));
        assert_eq!(keymap.feed("-"), Outcome::Action(Action::VolumeDown));
        assert_eq!(keymap.feed("="), Outcome::Action(Action::VolumeUp));
        assert_eq!(keymap.feed("shift-="), Outcome::Action(Action::VolumeUp));
        assert_eq!(keymap.feed("+"), Outcome::Action(Action::VolumeUp));

        assert_eq!(
            token_from_qt(0x4A, QT_CTRL | QT_SHIFT).as_deref(),
            Some("ctrl-shift-j")
        );
        assert_eq!(token_from_qt(0x2D, 0).as_deref(), Some("-"));
        assert_eq!(token_from_qt(0x3D, QT_SHIFT).as_deref(), Some("shift-="));
        assert_eq!(token_from_qt(0x2B, 0).as_deref(), Some("+"));
    }

    #[test]
    fn queue_reorder_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("ctrl-shift-j"),
            Outcome::Action(Action::QueueDown)
        );
        assert_eq!(
            keymap.feed("ctrl-shift-k"),
            Outcome::Action(Action::QueueUp)
        );
        assert_eq!(keymap.feed("shift-j"), Outcome::Action(Action::ScopeNext));
        assert_eq!(keymap.feed("shift-k"), Outcome::Action(Action::ScopePrev));
        // ARC-43: Alt chords belong to the quick-action overlay; the keymap
        // must not claim them.
        assert_eq!(keymap.feed("alt-j"), Outcome::None);
        assert_eq!(keymap.feed("alt-k"), Outcome::None);
    }

    #[test]
    fn album_order_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("shift-l"), Outcome::Action(Action::OrderNext));
        assert_eq!(keymap.feed("shift-h"), Outcome::Action(Action::OrderPrev));
    }

    #[test]
    fn layout_toggle_keys() {
        let mut keymap = Keymap::new();
        assert_eq!(
            keymap.feed("shift-c"),
            Outcome::Action(Action::LayoutToggle)
        );
        assert_eq!(token_from_qt(0x43, QT_SHIFT).as_deref(), Some("shift-c"));
    }

    #[test]
    fn vi7_counts_accumulate_and_ride_the_action() {
        // `3 j`.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.count(), 3);
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert_eq!(keymap.take_count(), 3);
        assert_eq!(keymap.count(), 0, "the count clears with the action");
        assert_eq!(keymap.take_count(), 0, "read once");

        // `10 l` — multi-digit.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("1"), Outcome::Pending);
        assert_eq!(keymap.feed("0"), Outcome::Pending);
        assert_eq!(keymap.count(), 10);
        assert_eq!(keymap.feed("l"), Outcome::Action(Action::MoveRight));
        assert_eq!(keymap.take_count(), 10);

        // `5 G` — the count rides a shift binding too.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("5"), Outcome::Pending);
        assert_eq!(keymap.feed("shift-g"), Outcome::Action(Action::Bottom));
        assert_eq!(keymap.take_count(), 5);

        // A lone leading zero has no motion to mean and starts nothing.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("0"), Outcome::None);
        assert_eq!(keymap.count(), 0);

        // A plain action reports no count.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("j"), Outcome::Action(Action::MoveDown));
        assert_eq!(keymap.take_count(), 0);
    }

    #[test]
    fn vi7_counts_clear_on_timeout_and_reset() {
        // A lone count times out silently.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed_timeout(), Outcome::None);
        assert_eq!(keymap.count(), 0);

        // Esc: the count dies with the sequence it was waiting on.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed("escape"), Outcome::Action(Action::Back));
        assert_eq!(keymap.take_count(), 3, "the count rides the fired action");
        assert_eq!(keymap.count(), 0);

        // reset() drops everything (context switches).
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("2"), Outcome::Pending);
        keymap.reset();
        assert_eq!(keymap.count(), 0);
        assert_eq!(keymap.take_count(), 0);
    }

    #[test]
    fn vi7_digits_do_not_join_sequences() {
        // `3 g g` works as count + the gg sequence.
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("3"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Pending);
        assert_eq!(keymap.feed("g"), Outcome::Action(Action::Top));
        assert_eq!(keymap.take_count(), 3);

        // A shifted digit is a symbol, not a count (`#` etc. stay unbound).
        assert_eq!(token_from_qt(0x31, QT_SHIFT).as_deref(), Some("shift-1"));
    }

    #[test]
    fn page_and_extend_motions() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.feed("ctrl-d"), Outcome::Action(Action::PageDown));
        assert_eq!(keymap.feed("ctrl-u"), Outcome::Action(Action::PageUp));
        assert_eq!(
            keymap.feed("shift-down"),
            Outcome::Action(Action::ExtendDown)
        );
        assert_eq!(keymap.feed("shift-up"), Outcome::Action(Action::ExtendUp));
        assert_eq!(
            keymap.feed("shift-left"),
            Outcome::Action(Action::ExtendLeft)
        );
        assert_eq!(
            keymap.feed("shift-right"),
            Outcome::Action(Action::ExtendRight)
        );
        assert_eq!(
            token_from_qt(QT_KEY_DOWN, QT_SHIFT).as_deref(),
            Some("shift-down")
        );
    }
}

mod library {
    use super::common::*;
    use crate::library::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await
    }

    #[tokio::test]
    async fn replace_round_trips_artists_albums_and_songs() {
        let pool = memory_pool().await;
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
            ..AlbumID3::default()
        };
        let song = Child {
            id: "s1".into(),
            album_id: Some("al1".into()),
            artist_id: Some("ar1".into()),
            title: "Everything in Its Right Place".into(),
            track: Some(1),
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
        assert_eq!(last_modified(&pool).await.unwrap(), 42);
    }

    #[tokio::test]
    async fn replace_reports_changed_albums_artists_and_covers() {
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
        sqlx::query(
            "INSERT INTO songs (id, title, artist) VALUES ('s1', 'Weird Fishes', 'Radiohead')",
        )
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
        assert_eq!(cycle_order("Year", 1), "Artist");
        assert_eq!(cycle_order("Artist", -1), "Year");
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
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
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
    async fn song_pages_follow_all_songs_order_without_materializing_it() {
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
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
    async fn artist_song_ids_and_file_info_feed_the_downloader() {
        let pool = memory_pool().await;
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
        let pool = memory_pool().await;
        for (id, album, artist) in [
            ("s1", "al1", "ar1"),
            ("s2", "al1", "ar1"),
            ("s3", "al2", "ar1"),
            ("s4", "al3", "ar2"),
        ] {
            sqlx::query(
                "INSERT INTO songs (id, title, album_id, artist_id) VALUES (?1, ?1, ?2, ?3)",
            )
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
        let pool = memory_pool().await;
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
        let pool = rt.block_on(memory_pool());
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
}

mod library_paths {
    use super::common::*;
    use crate::library::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await
    }

    #[tokio::test]
    async fn album_artist_id_resolves_the_owner() {
        let pool = memory_pool().await;
        sqlx::query("INSERT INTO albums (id, name, artist_id) VALUES ('al1', 'Kid A', 'ar1')")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            album_artist_id(&pool, "al1").await.unwrap().as_deref(),
            Some("ar1")
        );
        assert!(album_artist_id(&pool, "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn album_info_cache_and_cover_check_gate_round_trip() {
        let pool = memory_pool().await;
        sqlx::query("INSERT INTO albums (id, name) VALUES ('al1', 'Kid A')")
            .execute(&pool)
            .await
            .unwrap();

        assert!(album_info(&pool, "al1").await.unwrap().is_none());
        upsert_album_info(&pool, "al1", "notes").await.unwrap();
        let (notes, _) = album_info(&pool, "al1").await.unwrap().unwrap();
        assert_eq!(notes, "notes");

        invalidate_album_info(&pool, &["al1".to_string()])
            .await
            .unwrap();
        assert!(album_info(&pool, "al1").await.unwrap().is_none());

        upsert_album_info(&pool, "gone", "stale").await.unwrap();
        assert_eq!(purge_stale_album_info(&pool).await.unwrap(), 1);
        assert!(album_info(&pool, "gone").await.unwrap().is_none());

        assert!(cover_check_due(&pool, 3600).await.unwrap());
        touch_cover_check(&pool).await.unwrap();
        assert!(!cover_check_due(&pool, 3600).await.unwrap());
    }
}

mod lyrics {
    use super::common::*;
    use crate::api::{LyricLine, StructuredLyrics};
    use crate::lyrics::*;

    async fn lyrics_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_lyrics")).await
    }

    async fn library_pool() -> SqlitePool {
        crate::test::memory_pool(None).await
    }

    #[test]
    fn lrc_parsing_keeps_timestamps_and_skips_metadata() {
        let text = "[ar:Radiohead]\n[00:12.34]First line\n[01:02.5][01:04.00]Twice";
        let (synced, lines) = parse_lrc(text);
        assert!(synced);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].start_ms, Some(12_340));
        assert_eq!(lines[0].value, "First line");
        assert_eq!(lines[1].start_ms, Some(62_500));
        assert_eq!(lines[2].start_ms, Some(64_000));
    }

    #[test]
    fn plain_text_without_timestamps_is_unsynced() {
        let (synced, lines) = parse_lrc("just words\nno timing here");
        assert!(!synced);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.start_ms.is_none()));
    }

    #[test]
    fn structured_selection_prefers_synced_and_normalizes() {
        let list = LyricsList {
            structured_lyrics: vec![
                StructuredLyrics {
                    synced: false,
                    line: vec![LyricLine {
                        start: None,
                        value: "plain".to_string(),
                    }],
                    ..StructuredLyrics::default()
                },
                StructuredLyrics {
                    synced: true,
                    offset: Some(250),
                    line: vec![
                        LyricLine {
                            start: Some(1_000),
                            value: "first".to_string(),
                        },
                        LyricLine {
                            start: Some(2_000),
                            value: "second".to_string(),
                        },
                    ],
                    ..StructuredLyrics::default()
                },
            ],
        };

        let document = structured_document("s1", &list).unwrap();
        assert_eq!(document.source, "subsonic");
        assert!(document.synced);
        assert_eq!(document.offset_ms, 250);
        assert_eq!(document.lines.len(), 2);
        assert_eq!(document.lines[1].start_ms, Some(2_000));
    }

    #[test]
    fn partially_timed_entries_keep_their_known_starts() {
        let list = LyricsList {
            structured_lyrics: vec![StructuredLyrics {
                synced: true,
                line: vec![
                    LyricLine {
                        start: Some(1_000),
                        value: "timed".to_string(),
                    },
                    LyricLine {
                        start: None,
                        value: "untimed".to_string(),
                    },
                ],
                ..StructuredLyrics::default()
            }],
        };

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines[0].start_ms, Some(1_000));
        assert_eq!(document.lines[1].start_ms, None);
    }

    #[test]
    fn server_unsynced_flag_does_not_hide_line_timing() {
        // Some providers mark the entry unsynced while still sending `start`
        // on every line; the data wins over the flag.
        let list = LyricsList {
            structured_lyrics: vec![StructuredLyrics {
                synced: false,
                line: vec![
                    LyricLine {
                        start: Some(0),
                        value: "one".to_string(),
                    },
                    LyricLine {
                        start: Some(5_990),
                        value: "two".to_string(),
                    },
                ],
                ..StructuredLyrics::default()
            }],
        };

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines[1].start_ms, Some(5_990));
    }

    #[test]
    fn navidrome_synced_payload_is_not_downgraded() {
        // Trimmed getLyricsBySongId payload: synced true, empty-value lines
        // between the lyrics (the live probe for "Muscle Museum").
        let payload = serde_json::json!({
            "structuredLyrics": [{
                "displayArtist": "Muse",
                "displayTitle": "Muscle Museum",
                "lang": "xxx",
                "synced": true,
                "line": [
                    { "start": 0, "value": "Muscle Museum (soulwax remix)" },
                    { "start": 4840, "value": "" },
                    { "start": 5990, "value": "She had something to confess to" },
                    { "start": 10250, "value": "But you don’t have the time so" }
                ]
            }]
        });
        let list: LyricsList = serde_json::from_value(payload).unwrap();

        let document = structured_document("s1", &list).unwrap();
        assert!(document.synced);
        assert_eq!(document.lines.len(), 3, "empty lines are dropped");
        assert_eq!(document.lines[1].start_ms, Some(5_990));
        assert_eq!(document.lang.as_deref(), Some("xxx"));
    }

    #[test]
    fn legacy_lrc_text_is_parsed_when_timed() {
        let document = legacy_document("s1", "[00:01.00]first\n[00:02.50]second");
        assert_eq!(document.source, "legacy");
        assert!(document.synced);
        assert_eq!(document.lines[0].start_ms, Some(1_000));
        assert_eq!(document.lines[1].start_ms, Some(2_500));

        let plain = legacy_document("s2", "just words\nno timing here");
        assert!(!plain.synced);
        assert!(plain.lines.iter().all(|line| line.start_ms.is_none()));
    }

    #[test]
    fn unsynced_cache_expires_but_synced_does_not() {
        let old = now_secs() - UNSYNCED_TTL_SECS - 1;
        let fresh = now_secs();

        assert!(cache_is_stale(&Document::plain("s", "legacy", "a"), old));
        assert!(!cache_is_stale(&Document::plain("s", "legacy", "a"), fresh));
        assert!(cache_is_stale(
            &Document::missing("s"),
            now_secs() - MISS_TTL_SECS - 1
        ));
        assert!(!cache_is_stale(&Document::missing("s"), fresh));

        let mut synced = Document::plain("s", "subsonic", "a");
        synced.synced = true;
        assert!(!cache_is_stale(&synced, old), "synced hits never expire");
    }

    #[tokio::test]
    async fn synced_documents_round_trip_through_the_cache() {
        let pool = lyrics_pool().await;
        let document = Document {
            song_id: "s1".to_string(),
            source: "subsonic".to_string(),
            synced: true,
            lang: Some("eng".to_string()),
            offset_ms: 100,
            text: "one\ntwo".to_string(),
            lines: vec![
                Line {
                    start_ms: Some(500),
                    value: "one".to_string(),
                },
                Line {
                    start_ms: Some(1_500),
                    value: "two".to_string(),
                },
            ],
        };

        store(&pool, &document).await.unwrap();
        let (loaded, fetched_at) = load_cached(&pool, "s1").await.unwrap().unwrap();

        assert_eq!(loaded.source, "subsonic");
        assert!(loaded.synced);
        assert_eq!(loaded.offset_ms, 100);
        assert_eq!(loaded.lines.len(), 2);
        assert_eq!(loaded.lines[0].start_ms, Some(500));
        assert!(fetched_at > 0);
    }

    #[tokio::test]
    async fn plain_documents_round_trip_and_misses_too() {
        let pool = lyrics_pool().await;

        let document = Document::plain("s2", "legacy", "alpha\n\nbeta");
        store(&pool, &document).await.unwrap();
        let (loaded, _) = load_cached(&pool, "s2").await.unwrap().unwrap();
        assert!(!loaded.synced);
        assert_eq!(loaded.lines.len(), 2);
        assert_eq!(loaded.lines[1].value, "beta");

        let miss = Document::missing("s3");
        store(&pool, &miss).await.unwrap();
        let (loaded, _) = load_cached(&pool, "s3").await.unwrap().unwrap();
        assert_eq!(loaded.source, "none");
        assert!(!miss_expired(now_secs()));
    }

    #[tokio::test]
    async fn invalidate_drops_the_cached_document() {
        let pool = lyrics_pool().await;
        store(&pool, &Document::plain("s9", "subsonic", "one\ntwo"))
            .await
            .unwrap();

        invalidate(&pool, "s9").await.unwrap();
        assert!(load_cached(&pool, "s9").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalidate_songs_drops_only_the_changed_documents() {
        let pool = lyrics_pool().await;
        for id in ["s1", "s2", "s3"] {
            store(&pool, &Document::plain(id, "subsonic", "text"))
                .await
                .unwrap();
        }

        invalidate_songs(&pool, &["s1".to_string(), "s3".to_string()])
            .await
            .unwrap();
        assert!(load_cached(&pool, "s1").await.unwrap().is_none());
        assert!(load_cached(&pool, "s2").await.unwrap().is_some());
        assert!(load_cached(&pool, "s3").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn purge_removes_lyrics_for_songs_that_left_the_library() {
        let lyrics = lyrics_pool().await;
        let library = library_pool().await;
        sqlx::query("CREATE TABLE songs (id TEXT PRIMARY KEY)")
            .execute(&library)
            .await
            .unwrap();
        sqlx::query("INSERT INTO songs (id) VALUES ('keep')")
            .execute(&library)
            .await
            .unwrap();

        store(&lyrics, &Document::plain("keep", "legacy", "a"))
            .await
            .unwrap();
        store(&lyrics, &Document::plain("gone", "legacy", "b"))
            .await
            .unwrap();

        let removed = purge_with(&lyrics, &library).await.unwrap();
        assert_eq!(removed, 1);
        assert!(load_cached(&lyrics, "keep").await.unwrap().is_some());
        assert!(load_cached(&lyrics, "gone").await.unwrap().is_none());
    }

    #[test]
    fn backfill_candidates_skip_hits_and_fresh_misses() {
        let now = 1_000_000;
        let songs = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        let rows = vec![
            ("a".to_string(), "subsonic".to_string(), now - 10),
            ("b".to_string(), "none".to_string(), now - 10),
            (
                "c".to_string(),
                "none".to_string(),
                now - MISS_TTL_SECS - 10,
            ),
        ];

        // a = hit, b = fresh miss (skip); c = expired miss, d = unknown.
        assert_eq!(
            candidates(&songs, &rows, now, 10),
            vec!["c".to_string(), "d".to_string()]
        );
        // The cap bounds one sync pass.
        assert_eq!(candidates(&songs, &rows, now, 1).len(), 1);
    }
}

mod media {
    use super::common::*;
    use crate::media::*;

    async fn pool_with(migration: &str) -> SqlitePool {
        match migration {
            "media" => crate::test::memory_pool(sqlx::migrate!("./migrations_media")).await,
            _ => crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await,
        }
    }

    async fn media_pool() -> SqlitePool {
        pool_with("media").await
    }

    async fn library_pool() -> SqlitePool {
        pool_with("library").await
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("musika-media-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The streamed-download adoption: the `.part` file is renamed into the
    /// tier, its size recorded from the stream (not from a buffer) and a
    /// failed adopt leaves no ledger row behind.
    #[tokio::test]
    async fn streamed_blob_is_adopted_and_recorded() {
        let pool = media_pool().await;
        let dir = temp_dir("adopt");
        let temp = dir.join("download.part");
        std::fs::write(&temp, b"audio bytes").expect("part file");

        let path = store_in_dir(
            &pool,
            &dir,
            "s1",
            &temp,
            11,
            KIND_PERMANENT,
            "original",
            "fp",
            "flac",
            None,
        )
        .await
        .expect("adopt");

        let file = format!("{}.flac", md5_hex(b"s1").unwrap());
        assert_eq!(path, dir.join(&file));
        assert!(!temp.exists(), "the .part is consumed by the rename");
        assert_eq!(std::fs::read(&path).expect("blob"), b"audio bytes");

        let (size, kind, recorded): (i64, String, String) =
            sqlx::query_as("SELECT size, kind, file FROM media WHERE song_id = 's1'")
                .fetch_one(&pool)
                .await
                .expect("row");
        assert_eq!(size, 11);
        assert_eq!(kind, KIND_PERMANENT);
        assert_eq!(recorded, file);

        assert!(
            store_in_dir(
                &pool,
                &dir,
                "s2",
                &dir.join("absent.part"),
                0,
                KIND_PERMANENT,
                "original",
                "fp",
                "flac",
                None,
            )
            .await
            .is_err()
        );
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE song_id = 's2'")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(rows, 0, "a failed adopt must not record a row");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fingerprint_covers_every_input() {
        let base = fingerprint(Some(10), Some("mp3"), Some(320), "srv", "original");
        assert_eq!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(11), Some("mp3"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("ogg"), Some(320), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(192), "srv", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "other", "original")
        );
        assert_ne!(
            base,
            fingerprint(Some(10), Some("mp3"), Some(320), "srv", "mp3@192")
        );
        assert_eq!(
            fingerprint(None, None, None, "srv", "original"),
            "0||0|srv|original"
        );
    }

    #[test]
    fn tiers_split_config_from_cache() {
        assert_eq!(tier_for(KIND_PERMANENT), Tier::Config);
        assert_eq!(tier_for(KIND_RULE), Tier::Config);
        assert_eq!(tier_for(KIND_PLAYBACK), Tier::Cache);
        assert_eq!(tier_for(KIND_ROLLING), Tier::Cache);
    }

    #[tokio::test]
    async fn purge_drops_vanished_songs_and_rule_orphans() {
        let media = media_pool().await;
        let library = library_pool().await;

        sqlx::query("INSERT INTO songs (id, title) VALUES ('kept', 'A')")
            .execute(&library)
            .await
            .unwrap();
        sqlx::query("INSERT INTO media_rule (id, kind, ref_id, created_at) VALUES ('r1', 'artist', 'a1', 0)")
            .execute(&media)
            .await
            .unwrap();

        // A live rule-owned blob, a rule-orphan, and a vanished song's blob.
        for (song, rule) in [
            ("kept", Some("r1")),
            ("kept2", Some("dead")),
            ("gone", None),
        ] {
            record(
                &media,
                song,
                KIND_RULE,
                &format!("{song}.mp3"),
                1,
                "original",
                "fp",
                rule,
            )
            .await
            .unwrap();
        }

        let dbs = crate::db::Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers: library.clone(),
            lyrics: library.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            media: media.clone(),
            download: library.clone(),
            playlist: library.clone(),
        };
        assert_eq!(purge(&dbs).await.unwrap(), 2);

        let left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media ORDER BY song_id")
            .fetch_all(&media)
            .await
            .unwrap();
        assert_eq!(left, vec!["kept"]);
    }

    #[tokio::test]
    async fn rules_are_idempotent_and_remove_only_their_media() {
        let pool = media_pool().await;
        assert!(rule_for(&pool, "artist", "a1").await.unwrap().is_none());

        let id = set_rule(&pool, "artist", "a1").await.unwrap();
        assert_eq!(
            set_rule(&pool, "artist", "a1").await.unwrap(),
            id,
            "re-selecting never duplicates"
        );
        assert_eq!(
            rule_for(&pool, "artist", "a1").await.unwrap().unwrap().kind,
            "artist"
        );

        // One rule-owned blob, one explicit (permanent, no rule) blob.
        record(
            &pool,
            "s1",
            KIND_RULE,
            "s1.mp3",
            1,
            "original",
            "fp",
            Some(&id),
        )
        .await
        .unwrap();
        record(
            &pool,
            "s2",
            KIND_PERMANENT,
            "s2.mp3",
            1,
            "original",
            "fp",
            None,
        )
        .await
        .unwrap();

        assert_eq!(remove_rule_media(&pool, &id).await.unwrap(), 1);
        let left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, vec!["s2"], "explicit downloads survive rule removal");

        delete_rule(&pool, &id).await.unwrap();
        assert!(rule_for(&pool, "artist", "a1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn downloaded_counts_config_tier_content_only() {
        let pool = media_pool().await;
        record(
            &pool,
            "s1",
            KIND_RULE,
            "s1.mp3",
            1,
            "original",
            "fp",
            Some("r1"),
        )
        .await
        .unwrap();
        record(
            &pool,
            "s2",
            KIND_PLAYBACK,
            "s2.mp3",
            1,
            "original",
            "fp",
            None,
        )
        .await
        .unwrap();

        assert!(downloaded(&pool, "s1").await.unwrap());
        assert!(!downloaded(&pool, "s2").await.unwrap());
        assert!(!downloaded(&pool, "nope").await.unwrap());
    }

    #[tokio::test]
    async fn remove_songs_media_drops_config_tier_blobs_only() {
        let pool = media_pool().await;
        // s1: stored before rule ownership was recorded (the ledger names
        // the rule); s2: an explicit download outside the rule's songs;
        // s3: a playback-tier copy, which keeps its own purge.
        for (song, kind) in [
            ("s1", KIND_PERMANENT),
            ("s2", KIND_PERMANENT),
            ("s3", KIND_PLAYBACK),
        ] {
            record(
                &pool,
                song,
                kind,
                &format!("{song}.mp3"),
                1,
                "original",
                "fp",
                None,
            )
            .await
            .unwrap();
        }

        assert_eq!(
            remove_songs_media(&pool, &["s1".to_string(), "s3".to_string()])
                .await
                .unwrap(),
            1
        );
        let mut left: Vec<(String, String)> = sqlx::query_as("SELECT song_id, kind FROM media")
            .fetch_all(&pool)
            .await
            .unwrap();
        left.sort();
        assert_eq!(
            left,
            vec![
                ("s2".to_string(), KIND_PERMANENT.to_string()),
                ("s3".to_string(), KIND_PLAYBACK.to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn prune_uncovered_drops_scope_mismatches_only() {
        let media = media_pool().await;
        let library = library_pool().await;

        for (song, artist, album) in [
            ("s1", "a1", "al1"), // artist rule covers it
            ("s2", "a9", "al9"), // rule-owned but out of scope
            ("s3", "a1", "al1"), // explicit download (permanent)
            ("s4", "a5", "al7"), // album rule covers it
        ] {
            sqlx::query(
                "INSERT INTO songs (id, title, artist_id, album_id) VALUES (?1, ?1, ?2, ?3)",
            )
            .bind(song)
            .bind(artist)
            .bind(album)
            .execute(&library)
            .await
            .unwrap();
        }

        let artist_rule = set_rule(&media, "artist", "a1").await.unwrap();
        let album_rule = set_rule(&media, "album", "al7").await.unwrap();
        for (song, rule) in [
            ("s1", Some(artist_rule.as_str())),
            ("s2", Some(artist_rule.as_str())),
            ("s3", None),
            ("s4", Some(album_rule.as_str())),
        ] {
            let kind = if rule.is_some() {
                KIND_RULE
            } else {
                KIND_PERMANENT
            };
            record(
                &media,
                song,
                kind,
                &format!("{song}.mp3"),
                1,
                "original",
                "fp",
                rule,
            )
            .await
            .unwrap();
        }

        let dbs = crate::db::Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers: library.clone(),
            lyrics: library.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            media: media.clone(),
            download: library.clone(),
            playlist: library.clone(),
        };
        assert_eq!(prune_uncovered(&dbs).await.unwrap(), 1);

        let mut left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media")
            .fetch_all(&media)
            .await
            .unwrap();
        left.sort();
        assert_eq!(left, vec!["s1", "s3", "s4"]);
    }
}

mod mpris {
    use super::common::*;
    use crate::mpris::*;
    use zbus::zvariant::{ObjectPath, Value};

    #[test]
    fn track_paths_are_valid_object_paths() {
        assert_eq!(
            track_path("3PlpTsrmYRDfmuo3fe3PEJ"),
            "/org/musika/track/3PlpTsrmYRDfmuo3fe3PEJ"
        );
        assert_eq!(track_path("a-b.c"), "/org/musika/track/a_b_c");
        assert_eq!(track_path(""), "/org/mpris/MediaPlayer2/TrackList/NoTrack");
        assert!(ObjectPath::try_from(track_path("a-b.c")).is_ok());
    }

    #[test]
    fn metadata_carries_the_expected_roles() {
        let state = Snapshot {
            title: "Idioteque".to_string(),
            artist: "Radiohead".to_string(),
            album: "Kid A".to_string(),
            track_id: "abc123".to_string(),
            duration_us: 297_000_000,
            art_url: "file:///tmp/a.jpg".to_string(),
            ..Snapshot::default()
        };
        let map = metadata(&state);
        assert!(map.contains_key("mpris:trackid"));
        assert_eq!(map["mpris:length"], owned(Value::from(297_000_000i64)));
        assert_eq!(map["xesam:title"], owned(Value::from("Idioteque")));
        assert_eq!(
            map["xesam:artist"],
            owned(Value::from(vec!["Radiohead".to_string()]))
        );
        assert!(map.contains_key("mpris:artUrl"));
    }

    #[test]
    fn idle_state_exposes_the_no_track_placeholder() {
        let map = metadata(&Snapshot::default());
        assert_eq!(
            map["mpris:trackid"],
            owned(Value::from(
                ObjectPath::try_from("/org/mpris/MediaPlayer2/TrackList/NoTrack").unwrap()
            ))
        );
        assert!(!map.contains_key("mpris:length"));
    }

    #[test]
    fn signal_key_ignores_position_only_updates() {
        let mut state = Snapshot {
            title: "Idioteque".to_string(),
            ..Snapshot::default()
        };
        let before = state.signal_key();
        state.position_us = 42_000_000;
        assert_eq!(before, state.signal_key());

        state.volume = 0.5;
        assert_ne!(before, state.signal_key());
    }

    /// FIX-8: a flood beyond the capacity drops commands instead of growing
    /// the queue or blocking the bus thread.
    #[test]
    fn a_full_command_queue_drops_instead_of_blocking() {
        let (tx, mut rx) = mpsc::channel(2);
        queue_command(&tx, "mpris_play".to_string());
        queue_command(&tx, "mpris_next".to_string());
        queue_command(&tx, "mpris_previous".to_string());
        assert_eq!(rx.try_recv().as_deref(), Ok("mpris_play"));
        assert_eq!(rx.try_recv().as_deref(), Ok("mpris_next"));
        assert!(rx.try_recv().is_err(), "the excess command was dropped");
    }
}

mod player {
    use super::common::*;
    use crate::player::*;

    #[test]
    fn stale_end_file_does_not_record_an_error_while_a_load_is_pending() {
        let mut shared = Shared {
            pending_load: true,
            ..Shared::default()
        };
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "stop" }),
        );
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "loading failed" }),
        );
        assert!(shared.error.is_none());
    }

    #[test]
    fn start_file_clears_pending_and_the_stale_position() {
        let mut shared = Shared {
            position: 42.0,
            duration: 200.0,
            paused: true,
            loaded: true,
            idle: true,
            pending_load: true,
            ..Shared::default()
        };
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        assert_eq!(shared.position, 0.0);
        assert_eq!(shared.duration, 0.0);
        assert!(!shared.loaded);
        assert!(!shared.idle);
        assert!(!shared.pending_load);
    }

    #[test]
    fn stop_never_records_an_error_but_a_failure_does() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "stop" }),
        );
        assert!(shared.error.is_none());

        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "loading failed" }),
        );
        let error = shared.error.take().expect("error recorded");
        assert_eq!(error.message, "loading failed");
        assert!(!error.played, "a failed load never produced media");
    }

    #[test]
    fn an_error_after_playback_is_marked_as_played() {
        let mut shared = Shared::default();
        apply_event(&mut shared, &serde_json::json!({ "event": "start-file" }));
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "property-change", "name": "time-pos", "data": 7.5 }),
        );
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "end-file", "reason": "error", "file_error": "I/O error" }),
        );
        let error = shared.error.take().expect("error recorded");
        assert!(error.played);
    }

    #[test]
    fn playlist_pos_and_idle_are_mirrored() {
        let mut shared = Shared::default();
        apply_event(
            &mut shared,
            &serde_json::json!({ "event": "property-change", "name": "playlist-pos", "data": 1 }),
        );
        assert_eq!(shared.playlist_pos, Some(1));
        apply_event(&mut shared, &serde_json::json!({ "event": "file-loaded" }));
        assert!(shared.loaded);
        apply_event(&mut shared, &serde_json::json!({ "event": "idle" }));
        assert!(shared.idle);
    }

    fn track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: String::new(),
            album: String::new(),
            album_id: String::new(),
            favorite: false,
            duration: 1.0,
            url: String::new(),
            cover_art: String::new(),
            cover_url: String::new(),
        }
    }

    #[test]
    fn append_extends_the_queue_without_moving_the_cursor() {
        let mut player = Player::new();
        player.append(vec![track("s1"), track("s2")]);
        assert_eq!(player.len(), 2);
        assert_eq!(player.index(), 0);
        assert_eq!(player.current().map(|t| t.id.as_str()), Some("s1"));
        player.append(vec![track("s3")]);
        assert_eq!(player.len(), 3);
        assert_eq!(player.index(), 0);
    }

    #[test]
    fn request_ids_increase_by_one_per_command() {
        let mut player = Player::new();
        player.command(vec![json!("stop")]);
        player.command(vec![json!("stop")]);
        assert_eq!(player.request_id, 2);
    }

    /// The publisher takes an `Arc` snapshot, so an emit no longer deep-copies
    /// the whole queue. A row mutation while the snapshot is alive must leave
    /// the published rows untouched; afterwards the buffer is unique again and
    /// the next mutation edits it in place.
    #[test]
    fn a_queue_snapshot_is_copy_on_write() {
        let mut player = Player::new();
        player.append(vec![track("s0"), track("s1"), track("s2")]);
        let snapshot = player.queue_snapshot();
        assert!(Arc::ptr_eq(&snapshot, &player.queue));

        player.insert_many(0, vec![track("x")]);
        assert_eq!(snapshot.len(), 3, "the snapshot keeps its own row set");
        assert_eq!(player.len(), 4);
        assert!(!Arc::ptr_eq(&snapshot, &player.queue), "mutation copied");

        drop(snapshot);
        let buffer = Arc::as_ptr(&player.queue);
        player.append(vec![track("z")]);
        assert_eq!(
            Arc::as_ptr(&player.queue),
            buffer,
            "with no reader left, a mutation reuses the buffer"
        );
    }

    /// FIX-11 regression: `load_paused` pauses while the socket is still
    /// connecting (the command is dropped). Resolving the deferred start
    /// must not autoplay the restored queue.
    #[test]
    fn a_deferred_paused_start_stays_paused() {
        let mut player = Player::new();
        player.queue = Arc::new(vec![track("s1")]);
        player.deferred_start = true;
        player.state = State::Paused;
        player.finish_deferred_start();
        assert_eq!(player.state(), State::Paused);
        assert!(!player.deferred_start);
    }

    /// FIX-11 regression: a seek issued while connecting (restored playhead)
    /// survives the deferred rebuild.
    #[test]
    fn a_deferred_start_keeps_the_pending_playhead() {
        let mut player = Player::new();
        player.queue = Arc::new(vec![track("s1")]);
        player.deferred_start = true;
        player.state = State::Playing;
        player.shared().position = 42.0;
        player.finish_deferred_start();
        assert_eq!(player.state(), State::Playing);
        assert_eq!(player.position(), 42.0);
    }

    #[test]
    fn remove_many_drops_the_selection_and_keeps_the_current_track() {
        let mut player = Player::new();
        player.queue = Arc::new((0..=5).map(|i| track(&format!("s{i}"))).collect());
        player.index = 0;
        // Unsorted, with a duplicate and an out-of-range index: four rows
        // actually leave the queue.
        assert_eq!(player.remove_many(&[5, 2, 1, 4, 1, 9]), 4);
        assert_eq!(player.len(), 2);
        assert_eq!(player.current().map(|t| t.id.as_str()), Some("s0"));
        assert_eq!(player.index(), 0);
        assert_eq!(player.remove_many(&[1]), 1);
        assert_eq!(player.len(), 1);
        assert_eq!(player.remove_many(&[1, 9]), 0, "nothing valid to remove");
        assert_eq!(player.remove_many(&[0]), 1);
        assert!(player.queue.is_empty());
        assert_eq!(player.state(), State::Stopped);
    }

    fn ids(player: &Player) -> Vec<&str> {
        player
            .queue()
            .iter()
            .map(|track| track.id.as_str())
            .collect()
    }

    /// VI-8a: inserting above the current track shifts its index with it;
    /// the insert point clamps to the queue length.
    #[test]
    fn paste_above_the_current_track_keeps_it_current() {
        let mut player = Player::new();
        player.queue = Arc::new(vec![track("s0"), track("s1"), track("s2")]);
        player.index = 1;
        player.insert_many(0, vec![track("x"), track("y")]);
        assert_eq!(player.index(), 3);
        assert_eq!(player.current().map(|t| t.id.as_str()), Some("s1"));
        player.insert_many(99, vec![track("z")]);
        assert_eq!(ids(&player), ["x", "y", "s0", "s1", "s2", "z"]);
        assert_eq!(player.index(), 3);
        player.insert_many(0, Vec::new());
        assert_eq!(player.len(), 6, "an empty clipboard is a no-op");
    }

    /// FIX-1: reaping takes ownership of the spawned child (TERM + wait)
    /// and clears the recorded socket, so a retried start cannot orphan
    /// the process.
    #[test]
    fn reap_child_takes_the_child_and_the_down() {
        let mut player = Player::new();
        player.child = Some(Command::new("sleep").arg("1").spawn().unwrap());
        player.socket = Some(PathBuf::from("/tmp/nonexistent-musika.sock"));
        player.connect_deadline = Some(Instant::now() + Duration::from_secs(1));
        player.reap_child();
        assert!(player.child.is_none());
        assert!(player.socket.is_none());
        assert!(player.connect_deadline.is_none());
    }

    /// FIX-11: while the socket is coming up the start defers instead of
    /// sleeping on the caller; the same calendar retry never re-spawns,
    /// and `stop` during the deferral aborts the start.
    #[test]
    fn stop_during_a_deferred_start_aborts_it() {
        let mut player = Player::new();
        // Spawn mpv (connect pending) without connecting.
        if !matches!(player.ensure_ready(), Ready::Connecting) {
            return; // mpv not testable here; the live verify covers it.
        }
        player.state = State::Playing;
        player.deferred_start = true;
        player.stop();
        assert!(player.child.is_some(), "stop reuses the spawned child");
        assert!(!player.deferred_start);
        assert_eq!(player.state(), State::Stopped);
        player.pump();
        assert_eq!(player.state(), State::Stopped);
    }
}

mod playlists {
    use super::common::*;
    use crate::playlists::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(None).await
    }

    fn test_dbs(pools: &SqlitePool, playlist: &SqlitePool) -> Dbs {
        Dbs {
            credentials: pools.clone(),
            library: pools.clone(),
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

        // Unresolved stays unresolved.
        let entry = M3uEntry {
            path: "missing.flac".into(),
            artist: Some("Nobody".into()),
            title: Some("Nothing".into()),
            duration: None,
        };
        assert_eq!(matcher.resolve(&entry, dir), None);
    }
}

mod qml {
    use super::common::*;
    use crate::qml::*;

    /// The queue-covers payload goes straight into its final collections:
    /// all three subtrees, and missing keys default to empty.
    #[test]
    fn queue_covers_payload_deserializes_directly() {
        let song = crate::library::SongRow {
            id: "s1".to_string(),
            title: "Everything In Its Right Place".to_string(),
            artist: Some("Radiohead".to_string()),
            album: Some("Kid A".to_string()),
            album_id: Some("al1".to_string()),
            track: Some(1),
            duration: Some(240),
            suffix: Some("flac".to_string()),
            cover_art: Some("c1".to_string()),
            starred: None,
        };
        let json = serde_json::json!({
            "covers": {"c1": "file:///c1.jpg"},
            "albums": {"al1": "c1"},
            "songs": [song],
        })
        .to_string();

        let payload: QueueCoversPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(
            payload.covers.get("c1").map(String::as_str),
            Some("file:///c1.jpg")
        );
        assert_eq!(payload.albums.get("al1").map(String::as_str), Some("c1"));
        assert_eq!(payload.songs.len(), 1);
        assert_eq!(payload.songs[0].id, "s1");

        let empty: QueueCoversPayload = serde_json::from_str("{}").unwrap();
        assert!(empty.covers.is_empty() && empty.albums.is_empty() && empty.songs.is_empty());
    }

    /// Memory probe: the old `Value` + three subtree clones vs the typed
    /// `QueueCoversPayload` for a 20k-row queue payload.
    ///
    /// `cargo test test::qml::bench_queue_covers_payload -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_queue_covers_payload() {
        let songs: Vec<crate::library::SongRow> = (0..20_000)
            .map(|i| crate::library::SongRow {
                id: format!("s{i}"),
                title: format!("Track {i}"),
                artist: Some("Radiohead".to_string()),
                album: Some("Kid A".to_string()),
                album_id: Some(format!("al{}", i % 2000)),
                track: Some((i % 10) as i32 + 1),
                duration: Some(180 + i % 120),
                suffix: Some("flac".to_string()),
                cover_art: Some(format!("c{}", i % 2000)),
                starred: None,
            })
            .collect();
        let covers: HashMap<String, String> = (0..2000)
            .map(|i| (format!("c{i}"), format!("file:///covers/c{i}.jpg")))
            .collect();
        let albums: HashMap<String, String> = (0..2000)
            .map(|i| (format!("al{i}"), format!("c{i}")))
            .collect();
        let json = serde_json::json!({
            "covers": &covers,
            "albums": &albums,
            "songs": &songs,
        })
        .to_string();

        let envelope = QueueCoversEnvelope {
            covers: &covers,
            albums: &albums,
            songs: &songs,
        };
        let (_, value_tree_peak) = crate::test_alloc::measure(|| {
            serde_json::to_value(&envelope)
                .expect("envelope value")
                .to_string()
        });
        let (_, direct_peak) =
            crate::test_alloc::measure(|| serde_json::to_string(&envelope).expect("envelope json"));
        eprintln!(
            "queue covers envelope (20k songs): direct {} KiB | Value tree {} KiB",
            direct_peak / 1024,
            value_tree_peak / 1024,
        );
        assert!(
            direct_peak < value_tree_peak,
            "the typed envelope must not build a Value tree first"
        );

        let (_, clone_peak) = crate::test_alloc::measure(|| {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            let covers: HashMap<String, String> =
                serde_json::from_value(value["covers"].clone()).unwrap_or_default();
            let albums: HashMap<String, String> =
                serde_json::from_value(value["albums"].clone()).unwrap_or_default();
            let songs: Vec<crate::library::SongRow> =
                serde_json::from_value(value["songs"].clone()).unwrap_or_default();
            covers.len() + albums.len() + songs.len()
        });
        let (_, typed_peak) = crate::test_alloc::measure(|| {
            let payload: QueueCoversPayload = serde_json::from_str(&json).unwrap();
            payload.covers.len() + payload.albums.len() + payload.songs.len()
        });

        eprintln!(
            "queue covers payload (20k songs): typed {} KiB | Value + clones {} KiB",
            typed_peak / 1024,
            clone_peak / 1024,
        );
        assert!(
            typed_peak < clone_peak,
            "direct deserialization must beat the Value tree plus subtree clones"
        );
    }

    /// The queue stream must publish exactly the roles the drawer paints
    /// (QR-4) and the live favourite flag, without the old `Value` tree.
    #[test]
    fn queue_rows_stream_the_published_shape() {
        let track = crate::player::Track {
            id: "s1".to_string(),
            title: "Everything In Its Right Place".to_string(),
            artist: "Radiohead".to_string(),
            album: "Kid A".to_string(),
            album_id: "al1".to_string(),
            favorite: true,
            duration: 240.0,
            url: "https://server/stream?id=s1".to_string(),
            cover_art: "cov1".to_string(),
            cover_url: "file:///c.jpg".to_string(),
        };
        let starred: std::collections::HashSet<String> = ["s1".to_string()].into_iter().collect();
        let rows = QueueRows {
            queue: std::slice::from_ref(&track),
            starred: &starred,
        };

        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
        assert_eq!(parsed[0]["id"], "s1");
        assert_eq!(parsed[0]["title"], "Everything In Its Right Place");
        assert_eq!(parsed[0]["album_id"], "al1");
        assert_eq!(parsed[0]["starred"], true);
        assert_eq!(parsed[0]["duration"], 240.0);
        assert_eq!(parsed[0]["cover"], "file:///c.jpg");
        assert!(
            parsed[0].get("artist").is_none() && parsed[0].get("album").is_none(),
            "QR-4: the drawer never painted subtitle roles"
        );
    }
}

mod qml_download {
    use super::common::*;
    use crate::qml::download::*;

    async fn dbs() -> crate::db::Dbs {
        let pool = crate::test::memory_pool(None).await;
        sqlx::raw_sql(
            "CREATE TABLE songs (id TEXT PRIMARY KEY, album_id TEXT);
             CREATE TABLE download (
                 song_id TEXT PRIMARY KEY,
                 state TEXT NOT NULL,
                 error TEXT,
                 rule_id TEXT,
                 requested_at INTEGER NOT NULL DEFAULT 0,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );",
        )
        .execute(&pool)
        .await
        .unwrap();
        crate::db::Dbs {
            credentials: pool.clone(),
            library: pool.clone(),
            search: pool.clone(),
            covers: pool.clone(),
            media: pool.clone(),
            lyrics: pool.clone(),
            colors: pool.clone(),
            status: pool.clone(),
            artist_user: pool.clone(),
            download: pool.clone(),
            playlist: pool,
        }
    }

    #[tokio::test]
    async fn album_completion_requires_every_track() {
        let dbs = dbs().await;
        sqlx::raw_sql("INSERT INTO songs (id, album_id) VALUES ('s1', 'a1'), ('s2', 'a1')")
            .execute(&dbs.library)
            .await
            .unwrap();

        crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
            .await
            .unwrap();
        assert!(!album_complete(&dbs, "a1").await);

        crate::download::set_state(&dbs.download, "s2", crate::download::DONE, None)
            .await
            .unwrap();
        assert!(album_complete(&dbs, "a1").await);
        assert!(!album_complete(&dbs, "missing").await);
    }
}

mod queue {
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
        for (id, album_id, title, track) in
            [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
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
        for (id, album_id, title, track) in
            [("s1", "al1", "Everything", 1), ("s2", "al1", "Kid A", 2)]
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

    #[tokio::test]
    async fn shuffle_queue_resolves_every_unit_in_order() {
        let dbs = test_dbs().await;
        for (id, album) in [("s1", "al1"), ("s2", "al1"), ("s3", "al2"), ("s4", "al2")] {
            sqlx::query(
                "INSERT INTO songs (id, title, artist, album, album_id, duration)
                 VALUES (?1, ?1, 'A', 'X', ?2, 100)",
            )
            .bind(id)
            .bind(album)
            .execute(&dbs.library)
            .await
            .unwrap();
        }
        // Credentials exist so `tracks_json` can build stream URLs; the
        // pinned test key keeps the encrypt/decrypt round-trip off the config
        // dir (a sandbox `$HOME` is read-only).
        crate::crypto::use_test_key();
        crate::sqlite::save_credentials(&dbs.credentials, "u", "p", "http://s", "navidrome")
            .await
            .unwrap();

        // Songs resolve one track per unit, in the given order.
        let units: Vec<String> = ["s3", "s1", "s4", "s2"].map(String::from).to_vec();
        let json = shuffle_queue_json(&dbs, "songs", &units, false)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ids: Vec<&str> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["s3", "s1", "s4", "s2"]);

        // Albums expand whole, in the given order: al2's songs lead.
        let albums: Vec<String> = ["al2", "al1"].map(String::from).to_vec();
        let json = shuffle_queue_json(&dbs, "albums", &albums, false)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ids: Vec<&str> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 4);
        assert!(ids[..2].contains(&"s3") && ids[..2].contains(&"s4"));
        assert!(ids[2..].contains(&"s1") && ids[2..].contains(&"s2"));
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
}

mod search {
    use super::common::*;
    use crate::search::*;

    async fn rebuild(pool: &SqlitePool, rows: &[SongSearchRow]) -> anyhow::Result<()> {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM song_search")
            .execute(&mut *tx)
            .await?;
        crate::search::insert_search_rows(&mut tx, rows).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_search")).await
    }

    #[tokio::test]
    async fn search_matches_song_metadata() {
        let pool = memory_pool().await;

        let rows = vec![SongSearchRow {
            id: "1".to_string(),
            title: "Hello World".to_string(),
            artist: Some("The Testers".to_string()),
            album: Some("First".to_string()),
            duration: Some(200),
            play_count: Some(7),
        }];
        rebuild(&pool, &rows).await.unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].song_id, "1");
        assert_eq!(hits[0].artist.as_deref(), Some("The Testers"));
        assert_eq!(hits[0].duration, Some(200));
        assert_eq!(hits[0].play_count, 7);

        assert!(search(&pool, "missing", 10).await.unwrap().is_empty());
        assert!(search(&pool, "   ", 10).await.unwrap().is_empty());
    }

    fn row(id: &str, title: &str, plays: i64) -> SongSearchRow {
        SongSearchRow {
            id: id.to_string(),
            title: title.to_string(),
            artist: None,
            album: None,
            duration: Some(180),
            play_count: Some(plays),
        }
    }

    #[tokio::test]
    async fn most_played_orders_by_count_then_title() {
        let pool = memory_pool().await;
        rebuild(
            &pool,
            &[
                row("1", "Zebra", 0),
                row("2", "Alpha", 9),
                row("3", "Beta", 4),
                row("4", "Another", 9),
            ],
        )
        .await
        .unwrap();

        let hits = most_played(&pool, 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|hit| hit.song_id.as_str()).collect();
        // Two 9-play ties break by title, then the 4, then the 0.
        assert_eq!(ids, ["2", "4", "3", "1"]);
    }

    #[tokio::test]
    async fn play_count_boosts_equal_ranks() {
        let pool = memory_pool().await;
        // Same matched text => equal rank; the play count decides (S-9).
        rebuild(&pool, &[row("quiet", "Hello", 0), row("loud", "Hello", 40)])
            .await
            .unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|hit| hit.song_id.as_str()).collect();
        assert_eq!(ids, ["loud", "quiet"]);
    }

    #[tokio::test]
    async fn rebuild_from_library_pages_every_song() {
        // 600 rows spans more than one 500-row keyset page.
        let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
        sqlx::raw_sql(
            "INSERT INTO songs (id, title, artist, album, duration, play_count)
             SELECT printf('s%04d', x), 'Track ' || x, 'Artist', 'Album', 180, x % 5
             FROM (WITH RECURSIVE c(x) AS (
                       VALUES(1) UNION ALL SELECT x + 1 FROM c WHERE x < 600
                   ) SELECT x FROM c);",
        )
        .execute(&pool)
        .await
        .unwrap();

        rebuild_from_library(&pool).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 600);
        let hits = search(&pool, "599", 10).await.unwrap();
        assert_eq!(hits[0].song_id, "s0599");

        // A second pass replaces the table instead of appending to it.
        rebuild_from_library(&pool).await.unwrap();
        assert_eq!(count(&pool).await.unwrap(), 600);
    }

    #[tokio::test]
    async fn text_rank_leads_until_plays_are_large() {
        let pool = memory_pool().await;
        // bm25 magnitudes collapse to ~1e-6 in a two-row corpus, so use a
        // realistic size where the rank gap means something.
        let mut rows = vec![
            row("strong", "Hello Hello Hello Hello Hello", 0),
            row("weak", "Hello There", 3),
        ];
        for i in 0..20 {
            let id = format!("filler{i}");
            let title = format!("Filler {i}");
            rows.push(row(&id, &title, 0));
        }
        rebuild(&pool, &rows).await.unwrap();

        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(
            hits[0].song_id, "strong",
            "a strong match survives a small usage boost"
        );

        // Enough plays still overtake a better text match (the boost's job).
        rows[0].play_count = Some(0);
        rows[1].play_count = Some(60);
        rebuild(&pool, &rows).await.unwrap();
        let hits = search(&pool, "hello", 10).await.unwrap();
        assert_eq!(hits[0].song_id, "weak");
    }

    #[test]
    fn query_quotes_every_token_and_ignores_punctuation() {
        assert_eq!(fts_query("hello world"), "\"hello\"* \"world\"*");
        assert_eq!(fts_query(""), "");
        assert_eq!(clean_query("(hello) \"kid a\" [1979]"), "hello kid a 1979");
        assert_eq!(fts_query("(hello)"), "\"hello\"*");
    }

    #[tokio::test]
    async fn search_ignores_punctuation_in_the_query() {
        let pool = memory_pool().await;
        rebuild(&pool, &[row("1", "Hello World", 0)]).await.unwrap();

        for query in ["(hello)", "\"world\"", "[hello] world"] {
            let hits = search(&pool, query, 10).await.unwrap();
            assert_eq!(hits.len(), 1, "query {query:?}");
            assert_eq!(hits[0].song_id, "1");
        }
    }

    /// Memory probe: the paged FTS rebuild vs the old
    /// `song_search_rows()` + `rebuild(&rows)` path at 20k songs.
    ///
    /// `cargo test test::search::bench_fts_rebuild_peak -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_fts_rebuild_peak() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let pool = rt.block_on(crate::test::memory_pool(sqlx::migrate!(
            "./migrations_library"
        )));
        rt.block_on(async {
            sqlx::raw_sql(
                "INSERT INTO songs (id, title, artist, album, duration, play_count)
                 SELECT printf('s%06d', x), 'Track ' || x, 'Artist ' || (x % 200),
                        'Album ' || (x % 2000), 180, x % 5
                 FROM (WITH RECURSIVE c(x) AS (
                           VALUES(1) UNION ALL SELECT x + 1 FROM c WHERE x < 20000
                       ) SELECT x FROM c);",
            )
            .execute(&pool)
            .await
            .expect("seed songs");
        });

        let (_, old_peak) = crate::test_alloc::measure(|| {
            rt.block_on(async {
                let rows = crate::library::song_search_rows(&pool)
                    .await
                    .expect("collect rows");
                rebuild(&pool, &rows).await.expect("rebuild from vec");
            })
        });
        let (_, paged_peak) = crate::test_alloc::measure(|| {
            rt.block_on(rebuild_from_library(&pool))
                .expect("paged rebuild")
        });

        eprintln!(
            "FTS rebuild (20k songs): paged peak {} KiB | Vec + rebuild peak {} KiB",
            paged_peak / 1024,
            old_peak / 1024,
        );
        assert!(
            paged_peak < old_peak,
            "the paged rebuild must not hold the whole row set"
        );
        assert_eq!(rt.block_on(count(&pool)).unwrap(), 20_000);
    }
}

mod search_index {
    use super::common::*;
    use crate::search::index::*;

    fn new_index(
        songs: Vec<SongHit>,
        artists: Vec<ArtistRow>,
        albums: Vec<AlbumRow>,
    ) -> SearchIndex {
        SearchIndex::from_shared(songs.into_iter().map(Arc::new).collect(), artists, albums)
    }

    fn song(id: &str, title: &str, artist: &str, album: &str, plays: i64) -> SongHit {
        SongHit {
            id: id.to_string(),
            title: title.to_string(),
            artist: Some(artist.to_string()),
            album: Some(album.to_string()),
            duration: Some(200),
            play_count: plays,
            album_id: Some(format!("{album}-id")),
            cover_art: None,
            track: Some(1),
            disc_number: Some(1),
        }
    }

    fn artist(id: &str, name: &str) -> ArtistRow {
        ArtistRow {
            id: id.to_string(),
            name: name.to_string(),
            album_count: 1,
            cover_art: None,
            starred: None,
        }
    }

    fn album(id: &str, title: &str, artist: &str) -> AlbumRow {
        AlbumRow {
            id: id.to_string(),
            title: title.to_string(),
            artist: Some(artist.to_string()),
            year: Some(2000),
            song_count: 1,
            cover_art: None,
            starred: None,
            rating: 0,
        }
    }

    #[test]
    fn fuzzy_matches_across_title_artist_and_album() {
        let index = new_index(
            vec![song(
                "1",
                "Everything In Its Right Place",
                "Radiohead",
                "Kid A",
                0,
            )],
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(index.search("rdo", 10).songs.len(), 1);
        assert_eq!(index.search("kid", 10).songs.len(), 1);
        assert!(index.search("zzz", 10).songs.is_empty());
    }

    #[test]
    fn smartcase_and_diacritics_fold() {
        let index = new_index(
            vec![song("1", "Hello", "Björk", "Homogenic", 0)],
            vec![artist("a1", "Björk")],
            Vec::new(),
        );

        assert_eq!(index.search("bjork", 10).songs.len(), 1);
        assert_eq!(index.search("Bjork", 10).songs.len(), 1);
        assert!(index.search("BJORK", 10).songs.is_empty());
    }

    #[test]
    fn play_count_boosts_equal_text_scores() {
        let index = new_index(
            vec![
                song("quiet", "Hello", "A", "X", 0),
                song("loud", "Hello", "A", "X", 40),
            ],
            Vec::new(),
            Vec::new(),
        );

        let hits = index.search("hello", 10);
        let ids: Vec<&str> = hits.songs.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["loud", "quiet"]);
    }

    #[test]
    fn most_played_orders_by_count_then_title() {
        let index = new_index(
            vec![
                song("1", "Zebra", "A", "X", 0),
                song("2", "Alpha", "A", "X", 9),
                song("3", "Beta", "A", "X", 4),
                song("4", "Another", "A", "X", 9),
            ],
            Vec::new(),
            Vec::new(),
        );

        let hits = index.most_played(10);
        let ids: Vec<&str> = hits.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["2", "4", "3", "1"]);
    }

    #[test]
    fn album_tracks_follow_disc_and_track() {
        let mut second = song("b", "Second", "A", "X", 0);
        second.track = Some(2);
        let mut first = song("a", "First", "A", "X", 0);
        first.track = Some(1);
        let mut disc2 = song("c", "Disc Two", "A", "X", 0);
        disc2.disc_number = Some(2);
        disc2.track = Some(1);
        let index = new_index(vec![second, first, disc2], Vec::new(), Vec::new());

        let tracks = index.album_tracks("X-id", 10);
        let ids: Vec<&str> = tracks.iter().map(|song| song.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[test]
    fn artists_and_albums_match_their_own_lists() {
        let index = new_index(
            Vec::new(),
            vec![artist("a1", "Radiohead")],
            vec![album("al1", "Kid A", "Radiohead")],
        );

        let results = index.search("radio", 10);
        assert_eq!(results.artists.len(), 1);
        assert_eq!(results.albums.len(), 1);
        assert_eq!(results.artists[0].id, "a1");
        assert_eq!(results.albums[0].id, "al1");
    }

    /// The budget estimate is one aggregate scan: empty is free, and each row
    /// pays the fixed catalog cost plus five bytes per text byte (String +
    /// the UTF-32 haystack).
    #[tokio::test]
    async fn catalog_cost_charges_fixed_and_text_terms() {
        let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
        assert_eq!(catalog_cost(&pool).await.unwrap(), 0);

        sqlx::raw_sql(
            "INSERT INTO songs (id, title, artist, album, album_id, cover_art)
             VALUES ('s1', 'Title', 'Artist', 'Album', 'al1', 'cov')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let text =
            "s1".len() + "Title".len() + "Artist".len() + "Album".len() + "al1".len() + "cov".len();
        assert_eq!(
            catalog_cost(&pool).await.unwrap(),
            SONG_FIXED_COST + text * HAYSTACK_FACTOR
        );
        assert!(catalog_cost(&pool).await.unwrap() > 0);
    }

    /// M0 baseline: `cargo test --release test::search_index::bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "benchmark, run explicitly"]
    fn bench_100k_songs() {
        use std::time::Instant;

        // The fixture is built inside the tracked closure, so the input Vec's
        // buffer (consumed into `Arc<SongHit>`s) cancels out and the retained
        // count is the catalog itself.
        let ((index, elapsed, estimate), retained) = crate::test_alloc::live(|| {
            let songs: Vec<SongHit> = (0..100_000)
                .map(|i| {
                    song(
                        &format!("s{i}"),
                        &format!("Track {i}"),
                        "Radiohead",
                        "Kid A",
                        i % 100,
                    )
                })
                .collect();
            let songs_len = songs.len();
            let estimate = songs_len * SONG_FIXED_COST
                + songs
                    .iter()
                    .map(|hit| {
                        hit.id.len()
                            + hit.title.len()
                            + hit.artist.as_deref().map_or(0, str::len)
                            + hit.album.as_deref().map_or(0, str::len)
                            + hit.album_id.as_deref().map_or(0, str::len)
                            + hit.cover_art.as_deref().map_or(0, str::len)
                    })
                    .sum::<usize>()
                    * HAYSTACK_FACTOR;
            let start = Instant::now();
            let index = new_index(songs, Vec::new(), Vec::new());
            (index, start.elapsed(), estimate)
        });
        eprintln!(
            "index build (100k): {elapsed:?}, retained {:.1} MiB ({:.0} B/song), \
             budget estimate {:.1} MiB ({:.0} B/song)",
            retained as f64 / (1024.0 * 1024.0),
            retained as f64 / 100_000.0,
            estimate as f64 / (1024.0 * 1024.0),
            estimate as f64 / 100_000.0,
        );

        for query in ["rdo", "track", "radio", "zzz"] {
            let start = Instant::now();
            let hits = index.search(query, 20);
            eprintln!("{query}: {:?} ({} hits)", start.elapsed(), hits.songs.len());
        }
    }

    /// Build peak: the old `Vec<SongHit>` → `Arc` conversion vs the sync's
    /// streamed `Arc<SongHit>` path.
    ///
    /// `cargo test test::search_index::bench_index_build_peak -- --ignored --nocapture`
    #[test]
    #[ignore = "memory probe, run explicitly"]
    fn bench_index_build_peak() {
        let (_, vec_peak) = crate::test_alloc::measure(|| {
            let songs: Vec<SongHit> = (0..100_000)
                .map(|i| {
                    song(
                        &format!("s{i}"),
                        &format!("Track {i}"),
                        "Radiohead",
                        "Kid A",
                        i % 100,
                    )
                })
                .collect();
            new_index(songs, Vec::new(), Vec::new())
        });
        let (_, shared_peak) = crate::test_alloc::measure(|| {
            let songs: Vec<Arc<SongHit>> = (0..100_000)
                .map(|i| {
                    Arc::new(song(
                        &format!("s{i}"),
                        &format!("Track {i}"),
                        "Radiohead",
                        "Kid A",
                        i % 100,
                    ))
                })
                .collect();
            SearchIndex::from_shared(songs, Vec::new(), Vec::new())
        });

        eprintln!(
            "index build peak (100k): streamed Arcs {:.1} MiB | Vec<SongHit> {:.1} MiB",
            shared_peak as f64 / (1024.0 * 1024.0),
            vec_peak as f64 / (1024.0 * 1024.0),
        );
        assert!(
            shared_peak < vec_peak,
            "the streamed build must not hold both row forms"
        );
    }
}

mod status {
    use super::common::*;
    use crate::status::*;

    async fn memory_pool() -> SqlitePool {
        crate::test::memory_pool(sqlx::migrate!("./migrations_status")).await
    }

    #[tokio::test]
    async fn round_trips_and_last_write_wins() {
        let pool = memory_pool().await;
        assert!(load(&pool).await.unwrap().is_none());

        let first = SessionStatus {
            server_url: "https://music.example".to_string(),
            username: "demo".to_string(),
            album_id: Some("al1".to_string()),
            song_id: Some("s1".to_string()),
            queue: r#"["s1","s2","s3"]"#.to_string(),
            queue_index: 1,
            position: 61.5,
            volume: 42.0,
            muted: true,
            updated_at: 100,
        };
        save(&pool, &first).await.unwrap();
        assert_eq!(load(&pool).await.unwrap().as_ref(), Some(&first));

        // The single row is replaced, never duplicated.
        let second = SessionStatus {
            album_id: None,
            song_id: None,
            queue: String::new(),
            queue_index: 0,
            position: 0.0,
            volume: 70.0,
            muted: false,
            updated_at: 200,
            ..first.clone()
        };
        save(&pool, &second).await.unwrap();
        let loaded = load(&pool).await.unwrap().unwrap();
        assert_eq!(loaded, second);
        assert_eq!(loaded.album_id, None);
        assert_eq!(loaded.queue, "");
        assert_eq!(loaded.position, 0.0);
        assert_eq!(loaded.updated_at, 200);
    }
}

mod benchmark {
    use super::common::*;
    use super::memory_pool;
    use crate::browse::load_albums_json;

    fn dbs(library: &SqlitePool, media: &SqlitePool) -> Dbs {
        Dbs {
            credentials: library.clone(),
            library: library.clone(),
            search: library.clone(),
            covers: media.clone(),
            media: media.clone(),
            lyrics: media.clone(),
            colors: library.clone(),
            status: library.clone(),
            artist_user: library.clone(),
            download: media.clone(),
            playlist: library.clone(),
        }
    }

    /// Database open + embedded migrations, against SQLite in memory.
    ///
    /// `cargo test test::benchmark::bench_database_open -- --ignored --nocapture`
    #[ignore = "benchmark, run explicitly"]
    #[tokio::test]
    async fn bench_database_open() {
        let started = Instant::now();
        let library = memory_pool(sqlx::migrate!("./migrations_library")).await;
        let media = memory_pool(sqlx::migrate!("./migrations_media")).await;
        let search = memory_pool(sqlx::migrate!("./migrations_library")).await;
        println!(
            "[bench] database.open (memory + migrations): {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        drop((library, media, search));
    }

    struct AlbumSeed {
        id: String,
        name: String,
    }

    /// Album list read -> cover map -> JSON payload, on a seeded library.
    ///
    /// `cargo test test::benchmark::bench_albums_load -- --ignored --nocapture`
    #[ignore = "benchmark, run explicitly"]
    #[tokio::test]
    async fn bench_albums_load() {
        const ALBUMS: usize = 3_000;

        let library = memory_pool(sqlx::migrate!("./migrations_library")).await;
        let media = memory_pool(sqlx::migrate!("./migrations_media")).await;
        let rows: Vec<AlbumSeed> = (0..ALBUMS)
            .map(|i| AlbumSeed {
                id: format!("al{i}"),
                name: format!("Album {i:05}"),
            })
            .collect();
        let mut tx = library.begin().await.unwrap();
        crate::cache::insert_many(
            &mut tx,
            "INSERT INTO albums (id, artist_id, name, artist, year, song_count, duration, cover_art) ",
            &rows,
            |mut query, row| {
                query
                    .push_bind(&row.id)
                    .push_bind("ar1")
                    .push_bind(&row.name)
                    .push_bind("Bench Artist")
                    .push_bind(2026_i64)
                    .push_bind(10_i64)
                    .push_bind(2_400_i64)
                    .push_bind("");
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let dbs = dbs(&library, &media);
        let started = Instant::now();
        let payload = load_albums_json(&dbs, "", "Artist", false).await.unwrap();
        println!(
            "[bench] albums.load ({ALBUMS} rows): {:.2} ms ({} bytes)",
            started.elapsed().as_secs_f64() * 1000.0,
            payload.len()
        );
    }
}
