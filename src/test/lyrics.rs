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
    let (synced, lines, offset_ms) = parse_lrc(text);
    assert!(synced);
    assert_eq!(offset_ms, 0);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].start_ms, Some(12_340));
    assert_eq!(lines[0].value, "First line");
    assert_eq!(lines[1].start_ms, Some(62_500));
    assert_eq!(lines[2].start_ms, Some(64_000));
}

#[test]
fn multi_tag_and_out_of_order_timestamps_are_sorted_stably() {
    // Multi-tag line with its tags out of order, plus file lines out of order:
    // the parser must emit playlist-order timestamps, not file order.
    let text = "[00:10][00:05]chorus\n[00:09]verse\n[00:01]intro";
    let (synced, lines, _) = parse_lrc(text);
    assert!(synced);
    let times: Vec<Option<i64>> = lines.iter().map(|line| line.start_ms).collect();
    assert_eq!(
        times,
        vec![Some(1_000), Some(5_000), Some(9_000), Some(10_000)],
        "sorted by start time, multi-tag line kept per-timestamp"
    );
    assert_eq!(lines[1].value, "chorus");
    assert_eq!(lines[3].value, "chorus");
}

#[test]
fn untimed_lines_are_preserved_after_the_timed_ones() {
    let (synced, lines, _) = parse_lrc("plain credit\n[00:05]timed\nanother credit");
    assert!(synced);
    assert_eq!(lines[0].start_ms, Some(5_000));
    assert_eq!(lines[1].value, "plain credit");
    assert_eq!(lines[1].start_ms, None);
    assert_eq!(lines[2].value, "another credit");
}

#[test]
fn lrc_offset_tag_shifts_every_timestamp() {
    let text = "[offset:+500]\n[00:01.00]first\n[00:02.00]second";
    let (synced, lines, offset_ms) = parse_lrc(text);
    assert!(synced);
    assert_eq!(offset_ms, 500);
    assert_eq!(lines[0].start_ms, Some(1_500));
    assert_eq!(lines[1].start_ms, Some(2_500));

    // A negative offset pulls lines earlier but never before the track start.
    let text = "[offset:-250]\n[00:01.00]first\n[00:00.10]intro";
    let (_, lines, offset_ms) = parse_lrc(text);
    assert_eq!(offset_ms, -250);
    assert_eq!(
        lines[0].start_ms,
        Some(0),
        "negative clamped to track start"
    );
    assert_eq!(lines[1].start_ms, Some(750));

    let (_, lines, _) = parse_lrc("[offset:0]\n[00:03.00]line");
    assert_eq!(lines[0].start_ms, Some(3_000));
}

#[test]
fn corrupt_offset_and_timestamps_are_ignored_without_overflow() {
    // Out-of-range offset tags are corruption, not a sync hint.
    let (_, lines, offset_ms) = parse_lrc("[offset:99999999999999999999]\n[00:01.00]line");
    assert_eq!(offset_ms, 0);
    assert_eq!(lines[0].start_ms, Some(1_000));

    // Oversized/negative timestamp tags must not parse (no wraparound).
    assert_eq!(parse_timestamp("9223372036854775807:00.000"), None);
    assert_eq!(parse_timestamp("999999999999999999999:00"), None);
    assert_eq!(parse_timestamp("-1:00"), None);
    assert_eq!(parse_timestamp("00:-5"), None);
    assert_eq!(parse_timestamp("00:01.-5"), None);

    let (synced, lines, _) = parse_lrc("[9223372036854775807:00.000]huge\nplain");
    assert!(!synced, "an unparsable tag is metadata, not a timestamp");
    assert!(lines.iter().all(|line| line.start_ms.is_none()));
}

#[test]
fn plain_text_without_timestamps_is_unsynced() {
    let (synced, lines, offset_ms) = parse_lrc("just words\nno timing here");
    assert!(!synced);
    assert_eq!(offset_ms, 0);
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
    // N-14: the provider offset is applied once to the reported line starts.
    assert_eq!(document.lines[0].start_ms, Some(1_250));
    assert_eq!(document.lines[1].start_ms, Some(2_250));
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
