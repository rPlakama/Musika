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
    sqlx::query(
        "INSERT INTO media_rule (id, kind, ref_id, created_at) VALUES ('r1', 'artist', 'a1', 0)",
    )
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
        sync: library.clone(),
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
        sqlx::query("INSERT INTO songs (id, title, artist_id, album_id) VALUES (?1, ?1, ?2, ?3)")
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
        sync: library.clone(),
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
