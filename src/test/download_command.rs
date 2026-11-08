//! Scoped download/removal command resolution and local removal effects.

use super::common::*;
use crate::download_command::{ResolvedIds, execute, resolve_ids, search_match_id};
use crate::ui::ipc_command::{CurrentTrack, DownloadMode, DownloadRequest, DownloadScope};

async fn test_dbs() -> Dbs {
    let library = crate::test::memory_pool(sqlx::migrate!("./migrations_library")).await;
    let media = crate::test::memory_pool(sqlx::migrate!("./migrations_media")).await;
    Dbs {
        credentials: library.clone(),
        library: library.clone(),
        sync: library.clone(),
        search: library.clone(),
        covers: media.clone(),
        media: media.clone(),
        lyrics: media.clone(),
        colors: library.clone(),
        status: library.clone(),
        artist_user: library.clone(),
        download: media.clone(),
        playlist: library,
    }
}

async fn seed_library(dbs: &Dbs) {
    sqlx::raw_sql(
        "INSERT INTO artists (id, name, album_count) VALUES
             ('ar1', 'Radiohead', 2),
             ('ar2', 'Muse', 1);
         INSERT INTO albums (id, artist_id, name, artist, year, song_count) VALUES
             ('al1', 'ar1', 'Kid A', 'Radiohead', 2000, 2),
             ('al2', 'ar1', 'Amnesiac', 'Radiohead', 2001, 1),
             ('al3', 'ar2', 'Absolution', 'Muse', 2003, 1);
         INSERT INTO songs
             (id, album_id, artist_id, title, artist, album, track, disc_number, duration)
         VALUES
             ('s1', 'al1', 'ar1', 'Everything In Its Right Place', 'Radiohead', 'Kid A', 1, 1, 240),
             ('s2', 'al1', 'ar1', 'Idioteque', 'Radiohead', 'Kid A', 2, 1, 300),
             ('s3', 'al2', 'ar1', 'Pyramid Song', 'Radiohead', 'Amnesiac', 1, 1, 240),
             ('s4', 'al3', 'ar2', 'Time Is Running Out', 'Muse', 'Absolution', 1, 1, 180);",
    )
    .execute(&dbs.library)
    .await
    .unwrap();
}

fn path_request(
    artist: &str,
    album: Option<&str>,
    track: Option<&str>,
    remove: bool,
) -> DownloadRequest {
    DownloadRequest {
        remove,
        mode: DownloadMode::Path {
            artist: artist.to_string(),
            album: album.map(str::to_string),
            track: track.map(str::to_string),
        },
    }
}

fn current(id: &str) -> CurrentTrack {
    CurrentTrack {
        id: id.to_string(),
        title: String::new(),
        artist: String::new(),
        album: String::new(),
        album_id: String::new(),
    }
}

fn test_blob(song_id: &str) -> String {
    format!(
        "musika-download-command-test-{}-{song_id}.bin",
        std::process::id()
    )
}

async fn download_state(dbs: &Dbs, song_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT state FROM download WHERE song_id = ?1")
        .bind(song_id)
        .fetch_optional(&dbs.download)
        .await
        .unwrap()
}

async fn media_rows(dbs: &Dbs, song_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE song_id = ?1")
        .bind(song_id)
        .fetch_one(&dbs.media)
        .await
        .unwrap()
}

async fn album_rules(dbs: &Dbs, album_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM media_rule WHERE kind = 'album' AND ref_id = ?1")
        .bind(album_id)
        .fetch_one(&dbs.media)
        .await
        .unwrap()
}

#[tokio::test]
async fn path_resolution_walks_artist_album_and_track() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;

    // Artist only: every song of the artist.
    match resolve_ids(&dbs, &path_request("radiohead", None, None, false), None)
        .await
        .unwrap()
    {
        ResolvedIds::Artist {
            artist_id,
            mut song_ids,
        } => {
            assert_eq!(artist_id, "ar1");
            song_ids.sort();
            assert_eq!(song_ids, ["s1", "s2", "s3"]);
        }
        other => panic!("expected artist resolution, got {other:?}"),
    }

    // Album only: every song of that album.
    match resolve_ids(
        &dbs,
        &path_request("Radiohead", Some("kid a"), None, false),
        None,
    )
    .await
    .unwrap()
    {
        ResolvedIds::Album {
            album_id,
            mut song_ids,
        } => {
            assert_eq!(album_id, "al1");
            song_ids.sort();
            assert_eq!(song_ids, ["s1", "s2"]);
        }
        other => panic!("expected album resolution, got {other:?}"),
    }

    // Track: one id, matched case-insensitively.
    assert_eq!(
        resolve_ids(
            &dbs,
            &path_request("Radiohead", Some("Kid A"), Some("IDIOTEQUE"), false),
            None,
        )
        .await
        .unwrap(),
        ResolvedIds::Track("s2".to_string())
    );
}

#[test]
fn search_payload_matching_prefers_exact_then_prefix_then_first() {
    let payload = serde_json::json!({
        "artists": [
            {"id": "ar-prefix", "name": "Muse Tribute"},
            {"id": "ar-exact", "name": "Muse"},
        ],
        "albums": [
            {"id": "al-prefix", "title": "Kid A Live"},
            {"id": "al-exact", "title": "Kid A"},
        ],
        "songs": [
            {"id": "s-prefix", "title": "Idioteque (Live)"},
            {"id": "s-exact", "title": "Idioteque"},
        ],
    });

    // Case-insensitive exact wins even when the row is not first.
    assert_eq!(
        search_match_id(&payload, DownloadScope::Artist, "MUSE").as_deref(),
        Some("ar-exact")
    );
    assert_eq!(
        search_match_id(&payload, DownloadScope::Album, "kid a").as_deref(),
        Some("al-exact")
    );
    assert_eq!(
        search_match_id(&payload, DownloadScope::Track, "IDIOTEQUE").as_deref(),
        Some("s-exact")
    );

    // A prefix tie keeps the first stable hit.
    assert_eq!(
        search_match_id(&payload, DownloadScope::Artist, "mus").as_deref(),
        Some("ar-prefix")
    );
    // Contains matches only where a prefix did not.
    assert_eq!(
        search_match_id(&payload, DownloadScope::Album, "live").as_deref(),
        Some("al-prefix")
    );
    // A non-textual fuzzy hit falls back to the engine's first row.
    assert_eq!(
        search_match_id(&payload, DownloadScope::Track, "zzz").as_deref(),
        Some("s-prefix")
    );
    assert_eq!(
        search_match_id(&payload, DownloadScope::Artist, "zzz").as_deref(),
        Some("ar-prefix")
    );
}

#[tokio::test]
async fn no_match_errors_name_the_query() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;

    let err = resolve_ids(&dbs, &path_request("Nobody", None, None, false), None)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "No artist matched “Nobody”");

    let err = resolve_ids(
        &dbs,
        &path_request("Radiohead", Some("Nope"), None, false),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "No album matched “Nope”");

    // An empty album name must never rank every candidate as a prefix hit.
    let err = resolve_ids(
        &dbs,
        &path_request("Radiohead", Some(""), None, false),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "No album matched “”");

    let err = resolve_ids(
        &dbs,
        &path_request("Radiohead", Some("Kid A"), Some("Nope"), false),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "No track matched “Nope”");

    let err = execute(
        &dbs,
        &DownloadRequest {
            remove: false,
            mode: DownloadMode::Current,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("No current track"));
}

#[tokio::test]
async fn remove_track_unmarks_and_deletes_media() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;
    crate::download::mark_queued(&dbs.download, "s1", None)
        .await
        .unwrap();
    crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s1",
        crate::media::KIND_PERMANENT,
        &test_blob("s1"),
        1,
        "original",
        "fp",
        None,
    )
    .await
    .unwrap();

    execute(
        &dbs,
        &DownloadRequest {
            remove: true,
            mode: DownloadMode::Current,
        },
        Some(&current("s1")),
    )
    .await
    .unwrap();

    assert_eq!(download_state(&dbs, "s1").await, None);
    assert_eq!(media_rows(&dbs, "s1").await, 0);
}

#[tokio::test]
async fn remove_album_turns_off_container_and_clears_its_marks() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;

    let rule = crate::media::set_rule(&dbs.media, "album", "al1")
        .await
        .unwrap();
    // Rule-owned download plus an explicit one on the same album.
    crate::download::mark_queued(&dbs.download, "s1", Some(&rule))
        .await
        .unwrap();
    crate::download::set_state(&dbs.download, "s1", crate::download::DONE, None)
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s1",
        crate::media::KIND_RULE,
        &test_blob("s1"),
        1,
        "original",
        "fp",
        Some(&rule),
    )
    .await
    .unwrap();
    crate::download::mark_queued(&dbs.download, "s2", None)
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s2",
        crate::media::KIND_PERMANENT,
        &test_blob("s2"),
        1,
        "original",
        "fp",
        None,
    )
    .await
    .unwrap();

    // A different album is outside the requested scope.
    crate::download::mark_queued(&dbs.download, "s3", None)
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s3",
        crate::media::KIND_PERMANENT,
        &test_blob("s3"),
        1,
        "original",
        "fp",
        None,
    )
    .await
    .unwrap();

    execute(
        &dbs,
        &path_request("Radiohead", Some("Kid A"), None, true),
        None,
    )
    .await
    .unwrap();

    assert_eq!(album_rules(&dbs, "al1").await, 0);
    assert_eq!(download_state(&dbs, "s1").await, None);
    assert_eq!(download_state(&dbs, "s2").await, None);
    assert_eq!(media_rows(&dbs, "s1").await, 0);
    assert_eq!(media_rows(&dbs, "s2").await, 0);
    assert!(download_state(&dbs, "s3").await.is_some());
    assert_eq!(media_rows(&dbs, "s3").await, 1);
}

#[tokio::test]
async fn remove_artist_turns_off_every_album_container() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;

    let kid_a = crate::media::set_rule(&dbs.media, "album", "al1")
        .await
        .unwrap();
    let amnesiac = crate::media::set_rule(&dbs.media, "album", "al2")
        .await
        .unwrap();
    crate::download::mark_queued(&dbs.download, "s1", Some(&kid_a))
        .await
        .unwrap();
    crate::download::mark_queued(&dbs.download, "s3", Some(&amnesiac))
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s1",
        crate::media::KIND_RULE,
        &test_blob("s1"),
        1,
        "original",
        "fp",
        Some(&kid_a),
    )
    .await
    .unwrap();
    crate::media::record(
        &dbs.media,
        "s3",
        crate::media::KIND_RULE,
        &test_blob("s3"),
        1,
        "original",
        "fp",
        Some(&amnesiac),
    )
    .await
    .unwrap();
    crate::download::mark_queued(&dbs.download, "s2", None)
        .await
        .unwrap();

    // Another artist stays untouched.
    crate::download::mark_queued(&dbs.download, "s4", None)
        .await
        .unwrap();
    crate::media::record(
        &dbs.media,
        "s4",
        crate::media::KIND_PERMANENT,
        &test_blob("s4"),
        1,
        "original",
        "fp",
        None,
    )
    .await
    .unwrap();

    execute(&dbs, &path_request("Radiohead", None, None, true), None)
        .await
        .unwrap();

    assert_eq!(album_rules(&dbs, "al1").await, 0);
    assert_eq!(album_rules(&dbs, "al2").await, 0);
    for song_id in ["s1", "s2", "s3"] {
        assert_eq!(download_state(&dbs, song_id).await, None, "{song_id}");
        assert_eq!(media_rows(&dbs, song_id).await, 0, "{song_id}");
    }
    assert!(download_state(&dbs, "s4").await.is_some());
    assert_eq!(media_rows(&dbs, "s4").await, 1);
}

#[tokio::test]
async fn download_tries_every_song_and_reports_failures() {
    let dbs = test_dbs().await;
    seed_library(&dbs).await;

    // No credentials are seeded, so each fetch fails before touching the
    // network. The resolver still reaches every album song.
    let err = execute(
        &dbs,
        &path_request("Radiohead", Some("Kid A"), None, false),
        None,
    )
    .await
    .unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("No songs were downloaded"), "{message}");
    assert!(message.contains("2/2 failed"), "{message}");
    assert_eq!(
        download_state(&dbs, "s1").await.as_deref(),
        Some(crate::download::ERROR)
    );
    assert_eq!(
        download_state(&dbs, "s2").await.as_deref(),
        Some(crate::download::ERROR)
    );
}
