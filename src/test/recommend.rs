use super::common::*;

/// A small library whose genre/era overlap exercises the heuristic: the seed
/// is a 1977 Rock album by an established artist; Pink Floyd (Rock, late 70s,
/// several songs) must outrank a one-song Rock artist and a same-era MPB
/// artist.
async fn fixture() -> Dbs {
    let dbs = isolated_dbs().await;
    for (id, name) in [
        ("ar1", "David Bowie"),
        ("ar2", "Pink Floyd"),
        ("ar3", "King Crimson"),
        ("ar4", "Nirvana"),
        ("ar5", "Milton Nascimento"),
        ("ar6", "One Hit Wonder"),
        ("ar7", "Untagged Band"),
    ] {
        sqlx::query("INSERT INTO artists (id, name, album_count) VALUES (?1, ?2, 1)")
            .bind(id)
            .bind(name)
            .execute(&dbs.library)
            .await
            .unwrap();
    }
    for (id, artist, name, year, genre) in [
        ("al1", "ar1", "Heroes", 1977, Some("Rock")),
        ("al2", "ar1", "Low", 1977, Some("Rock")),
        ("al3", "ar2", "The Dark Side of the Moon", 1973, Some("Rock")),
        ("al4", "ar2", "The Wall", 1979, Some("Rock")),
        ("al5", "ar3", "In the Court of the Crimson King", 1969, Some("Rock")),
        ("al6", "ar4", "Nevermind", 1991, Some("Alternative Rock")),
        ("al7", "ar5", "Clube da Esquina", 1972, Some("MPB")),
        ("al8", "ar6", "One Song", 1977, Some("Rock")),
        ("al9", "ar7", "Untagged", 0, None),
    ] {
        sqlx::query(
            "INSERT INTO albums (id, artist_id, name, artist, year, genre, song_count)
             VALUES (?1, ?2, ?3, ?2, ?4, ?5, 1)",
        )
        .bind(id)
        .bind(artist)
        .bind(name)
        .bind(year)
        .bind(genre)
        .execute(&dbs.library)
        .await
        .unwrap();
    }
    let songs = [
        ("s1", "al1", "ar1", "Heroes", 1977, Some("Rock"), 1),
        ("s1b", "al2", "ar1", "Speed of Life", 1977, Some("Rock"), 2),
        ("s2", "al3", "ar2", "Time", 1973, Some("Rock"), 5),
        ("s3", "al3", "ar2", "Money", 1973, Some("Rock"), 8),
        ("s4", "al4", "ar2", "Another Brick in the Wall", 1979, Some("Rock"), 9),
        ("s5", "al5", "ar3", "21st Century Schizoid Man", 1969, Some("Rock"), 4),
        ("s6", "al6", "ar4", "Smells Like Teen Spirit", 1991, Some("Alternative Rock"), 7),
        ("s7", "al7", "ar5", "Travessia", 1972, Some("MPB"), 3),
        ("s7b", "al7", "ar5", "Nuvem Cigana", 1972, Some("MPB"), 2),
        ("s8", "al8", "ar6", "Only Hit", 1977, Some("Rock"), 1),
        ("s9", "al9", "ar7", "No Tags", 0, None, 0),
    ];
    for (id, album, artist, title, year, genre, play_count) in songs {
        sqlx::query(
            "INSERT INTO songs
                (id, album_id, artist_id, title, artist, album, track, disc_number,
                 year, genre, duration, play_count)
             VALUES (?1, ?2, ?3, ?4, ?3, ?5, 1, 1, ?6, ?7, 240, ?8)",
        )
        .bind(id)
        .bind(album)
        .bind(artist)
        .bind(title)
        .bind(album)
        .bind(year)
        .bind(genre)
        .bind(play_count)
        .execute(&dbs.library)
        .await
        .unwrap();
    }
    dbs
}

#[tokio::test]
async fn local_artists_rank_genre_and_era_over_one_hit_song() {
    let dbs = fixture().await;
    let order = crate::recommend::similar_artists_for_test(&dbs, "song", "s1")
        .await
        .unwrap();
    let position = |artist: &str| order.iter().position(|id| id == artist);
    let pink_floyd = position("ar2").expect("Pink Floyd should be a neighbour");
    let milton = position("ar5").expect("same-era MPB artist should still be nearby");
    let one_hit = position("ar6");
    assert_eq!(
        pink_floyd, 0,
        "the genre+era+catalog match must lead: {order:?}"
    );
    assert!(pink_floyd < milton, "genre overlap must outrank era alone");
    assert!(
        one_hit.is_none_or(|one_hit| pink_floyd < one_hit),
        "a one-song artist must not outrank a real catalog match"
    );
}

#[tokio::test]
async fn cached_provider_graph_leads_the_local_ranking() {
    let dbs = fixture().await;
    crate::library::upsert_artist_similar(
        &dbs.library,
        "ar1",
        &[crate::library::SimilarArtist {
            id: "ar5".to_string(),
            name: "Milton Nascimento".to_string(),
        }],
    )
    .await
    .unwrap();

    let order = crate::recommend::similar_artists_for_test(&dbs, "song", "s1")
        .await
        .unwrap();
    assert_eq!(order.first().map(String::as_str), Some("ar5"));
    assert!(order.contains(&"ar2".to_string()));
}

#[test]
fn genre_tokens_split_tags() {
    assert_eq!(
        crate::recommend::genre_tokens_for_test("Alternativo & Indie"),
        vec!["alternativo".to_string(), "indie".to_string()]
    );
    assert_eq!(
        crate::recommend::genre_tokens_for_test("Rock progressivo"),
        vec!["progressivo".to_string(), "rock".to_string()]
    );
    assert!(crate::recommend::genre_tokens_for_test("").is_empty());
}

#[tokio::test]
async fn expansion_is_deterministic_and_excludes_seed_album() {
    let dbs = fixture().await;
    crate::radio::clear_cache();
    let first = crate::recommend::similar_songs(&dbs, "song", "s1", 8, false, 42)
        .await
        .unwrap();
    let second = crate::recommend::similar_songs(&dbs, "song", "s1", 8, false, 42)
        .await
        .unwrap();
    let ids: Vec<&str> = first.iter().map(|song| song.id.as_str()).collect();
    let again: Vec<&str> = second.iter().map(|song| song.id.as_str()).collect();
    assert_eq!(ids, again, "a fixed RNG seed must be reproducible");
    assert!(!ids.contains(&"s1"), "the seed song is never in its own radio");
    assert!(
        first
            .iter()
            .all(|song| song.album_id.as_deref() != Some("al1")),
        "the seed album is excluded"
    );
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "no duplicate songs");

    let album = crate::recommend::similar_songs(&dbs, "album", "al1", 8, false, 7)
        .await
        .unwrap();
    assert!(
        album
            .iter()
            .all(|song| song.album_id.as_deref() != Some("al1")),
        "album radio leaves the seed album out"
    );
    assert!(!album.is_empty());
    crate::radio::clear_cache();
}

#[tokio::test]
async fn offline_expansion_only_returns_downloaded_songs() {
    let _offline = lock_offline_catalog().await;
    let dbs = fixture().await;
    for song in ["s2", "s3", "s6"] {
        sqlx::query(
            "INSERT INTO download (song_id, state, requested_at, updated_at)
             VALUES (?1, 'done', 0, 0)",
        )
        .bind(song)
        .execute(&dbs.download)
        .await
        .unwrap();
    }
    crate::offline::clear_cache();
    let rows = crate::recommend::similar_songs(&dbs, "song", "s1", 10, true, 3)
        .await
        .unwrap();
    let allowed: HashSet<&str> = ["s2", "s3", "s6"].into_iter().collect();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|song| allowed.contains(song.id.as_str())),
        "offline radio must only offer downloaded songs: {:?}",
        rows.iter().map(|song| &song.id).collect::<Vec<_>>()
    );
    crate::offline::clear_cache();
}

#[tokio::test]
async fn playlist_affinity_counts_co_occurrence() {
    let dbs = fixture().await;
    sqlx::raw_sql(
        "CREATE TABLE playlist_song (
             playlist_id TEXT NOT NULL,
             position    INTEGER NOT NULL,
             song_id     TEXT,
             title       TEXT NOT NULL,
             artist      TEXT,
             album       TEXT,
             duration    INTEGER,
             cover_art   TEXT,
             PRIMARY KEY (playlist_id, position)
         );",
    )
    .execute(&dbs.playlist)
    .await
    .unwrap();
    for (position, song, title) in [(0, "s1", "Heroes"), (1, "s7", "Travessia")] {
        sqlx::query(
            "INSERT INTO playlist_song (playlist_id, position, song_id, title)
             VALUES ('pl1', ?1, ?2, ?3)",
        )
        .bind(position)
        .bind(song)
        .bind(title)
        .execute(&dbs.playlist)
        .await
        .unwrap();
    }
    let affinity = crate::recommend::playlist_affinity_for_test(&dbs, "ar1")
        .await
        .unwrap();
    assert_eq!(affinity.get("ar5").copied(), Some(1.0));
    assert!(
        !affinity.contains_key("ar1"),
        "the seed artist is not its own neighbour"
    );
}

#[tokio::test]
async fn artist_without_metadata_still_fills_a_radio() {
    let dbs = fixture().await;
    let rows = crate::recommend::similar_songs(&dbs, "song", "s9", 5, false, 11)
        .await
        .unwrap();
    assert!(!rows.is_empty(), "the random top-up keeps radio non-empty");
    assert!(rows.iter().all(|song| song.id != "s9"));
    assert!(rows.len() <= 5);
}
