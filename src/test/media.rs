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
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A `Dbs` whose media/download/covers side lives on the media pool and the
/// library side on the library pool (the production split, see `db::init`).
fn test_dbs(media: SqlitePool, library: SqlitePool) -> crate::db::Dbs {
    crate::db::Dbs {
        credentials: library.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers: media.clone(),
        lyrics: media.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        media: media.clone(),
        download: media,
        playlist: library,
    }
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

    _ = std::fs::remove_dir_all(dir);
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

    let dbs = test_dbs(media.clone(), library.clone());
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

    // N-05: the ledger rows of the pruned songs must follow their blobs.
    for song in ["s1", "s2", "s4"] {
        crate::download::set_state(&media, song, crate::download::DONE, None)
            .await
            .unwrap();
    }

    let dbs = test_dbs(media.clone(), library.clone());
    assert_eq!(prune_uncovered(&dbs).await.unwrap(), 1);

    let mut left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media")
        .fetch_all(&media)
        .await
        .unwrap();
    left.sort();
    assert_eq!(left, vec!["s1", "s3", "s4"]);

    let mut ledger: Vec<String> =
        sqlx::query_scalar("SELECT song_id FROM download ORDER BY song_id")
            .fetch_all(&media)
            .await
            .unwrap();
    ledger.sort();
    assert!(
        !ledger.contains(&"s2".to_string()),
        "a pruned download must leave the offline ledger: {ledger:?}"
    );
}

#[tokio::test]
async fn playback_cache_cap_evicts_only_playback_rows() {
    let pool = media_pool().await;
    for (song, kind, size, used) in [
        ("old", KIND_PLAYBACK, 10, 1),
        ("new", KIND_PLAYBACK, 10, 3),
        ("download", KIND_PERMANENT, 100, 2),
    ] {
        record(
            &pool,
            song,
            kind,
            &format!("{song}.mp3"),
            size,
            "original",
            "fp",
            None,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE media SET last_used = ?2 WHERE song_id = ?1")
            .bind(song)
            .bind(used)
            .execute(&pool)
            .await
            .unwrap();
    }

    let removed = enforce_playback_cap(&pool, 15).await.unwrap();
    assert_eq!(removed, 1);
    let mut left: Vec<(String, String)> = sqlx::query_as("SELECT song_id, kind FROM media")
        .fetch_all(&pool)
        .await
        .unwrap();
    left.sort();
    assert_eq!(
        left,
        vec![
            ("download".to_string(), KIND_PERMANENT.to_string()),
            ("new".to_string(), KIND_PLAYBACK.to_string()),
        ]
    );
}

/// N-09: recency — not size — decides eviction. The old newest-first greedy
/// pass deleted the large mid-age blob and kept a smaller oldest one, i.e. it
/// did not behave like an LRU at all.
#[tokio::test]
async fn playback_cache_cap_evicts_the_oldest_rows_first() {
    let pool = media_pool().await;
    for (song, size, used) in [
        ("newest-large", 100, 30),
        ("mid", 10, 20),
        ("oldest-small", 5, 10),
    ] {
        record(
            &pool,
            song,
            KIND_PLAYBACK,
            &format!("{song}.mp3"),
            size,
            "original",
            "fp",
            None,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE media SET last_used = ?2 WHERE song_id = ?1")
            .bind(song)
            .bind(used)
            .execute(&pool)
            .await
            .unwrap();
    }

    // Total is 115 bytes; the cap keeps the 100-byte newest blob and must
    // evict strictly from the old end (oldest-small, then mid).
    let removed = enforce_playback_cap(&pool, 105).await.unwrap();
    assert_eq!(removed, 2);
    let left: Vec<String> = sqlx::query_scalar("SELECT song_id FROM media ORDER BY song_id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        left,
        vec!["newest-large"],
        "the newest blob survives; eviction is oldest-first"
    );
}

/// N-05: a crash's `active` rows are recovered to `error`, so the song
/// can be retried instead of being locked out forever.
#[tokio::test]
async fn reset_stale_downloads_recovers_active_rows_only() {
    let media = media_pool().await;
    let library = library_pool().await;
    for (song, state) in [
        ("stuck", crate::download::ACTIVE),
        ("done", crate::download::DONE),
        ("queued", crate::download::QUEUED),
    ] {
        crate::download::set_state(&media, song, state, None)
            .await
            .unwrap();
    }

    let dbs = test_dbs(media.clone(), library);
    assert_eq!(reset_stale_downloads(&dbs).await.unwrap(), 1);
    let (state, error): (String, Option<String>) =
        sqlx::query_as("SELECT state, error FROM download WHERE song_id = 'stuck'")
            .fetch_one(&media)
            .await
            .unwrap();
    assert_eq!(state, crate::download::ERROR);
    assert!(
        error.is_some(),
        "the recovery leaves a reason for the retry UI"
    );
    let done: String = sqlx::query_scalar("SELECT state FROM download WHERE song_id = 'done'")
        .fetch_one(&media)
        .await
        .unwrap();
    assert_eq!(done, crate::download::DONE);
}

/// N-04: the temp sweep must leave an in-flight `.part` alone and only remove
/// files old enough to be abandoned.
#[test]
fn temp_sweep_keeps_active_part_files_and_removes_stale_ones() {
    let dir = temp_dir("temp-sweep");
    let active = dir.join(".1.2.1.part");
    std::fs::write(&active, b"in-flight").unwrap();
    let stale_part = dir.join(".1.2.2.part");
    std::fs::write(&stale_part, b"abandoned").unwrap();
    let stale_tmp = dir.join(".1.2.3.tmp");
    std::fs::write(&stale_tmp, b"abandoned").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(11 * 60);
    for path in [&stale_part, &stale_tmp] {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open stale fixture");
        file.set_modified(old).expect("backdate fixture");
    }
    let kept = dir.join("blob.flac");
    std::fs::write(&kept, b"blob").unwrap();

    remove_temp_files(&dir);

    assert!(active.exists(), "an active `.part` must never be swept");
    assert!(!stale_part.exists(), "an abandoned `.part` is garbage");
    assert!(!stale_tmp.exists(), "an abandoned `.tmp` is garbage");
    assert!(kept.exists(), "the sweep only touches temp files");
    _ = std::fs::remove_dir_all(dir);
}

/// N-05: the media upsert refuses to downgrade a downloaded blob to the
/// purgeable playback cache, atomically inside the SQL statement.
#[tokio::test]
async fn record_refuses_downgrading_a_download_to_playback_cache() {
    let pool = media_pool().await;
    record(
        &pool,
        "s1",
        KIND_PERMANENT,
        "s1.flac",
        10,
        "original",
        "fp",
        None,
    )
    .await
    .unwrap();
    record(
        &pool,
        "s1",
        KIND_RULE,
        "s1.flac",
        10,
        "original",
        "fp",
        Some("r1"),
    )
    .await
    .unwrap();

    let error = record(
        &pool,
        "s1",
        KIND_PLAYBACK,
        "s1.flac",
        10,
        "original",
        "fp",
        None,
    )
    .await
    .expect_err("a download must not be replaced by a playback copy");
    assert!(
        format!("{error:#}").contains("refusing"),
        "the refusal must name itself, got: {error:#}"
    );
    let kind: String = sqlx::query_scalar("SELECT kind FROM media WHERE song_id = 's1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kind, KIND_RULE, "the config-tier row survives untouched");
}
