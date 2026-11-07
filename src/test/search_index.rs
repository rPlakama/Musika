use super::common::*;
use crate::search::index::*;

fn new_index(songs: Vec<SongHit>, artists: Vec<ArtistRow>, albums: Vec<AlbumRow>) -> SearchIndex {
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
        duration: 0,
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
    assert_eq!(index.search("r", 10).songs.len(), 1);
    assert_eq!(index.search("rdhd", 10).songs.len(), 1);
    assert_eq!(index.search("\"radiohead", 10).songs.len(), 1);
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

/// The post-first-frame warm-up builds a catalog that `ensure` then reuses.
#[tokio::test]
async fn warm_builds_a_reusable_catalog() {
    if !enabled() {
        return;
    }
    clear();
    let pool = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    warm(&pool).await.unwrap();
    assert!(ensure(&pool).await.unwrap().is_some());
    clear();
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
