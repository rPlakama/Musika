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
    assert_eq!(hits[0].id, "1");
    assert_eq!(hits[0].artist.as_deref(), Some("The Testers"));
    assert_eq!(hits[0].duration, Some(200));
    assert_eq!(hits[0].play_count, Some(7));

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
    let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
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
    let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
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
    assert_eq!(hits[0].id, "s0599");

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
        hits[0].id, "strong",
        "a strong match survives a small usage boost"
    );

    // Enough plays still overtake a better text match (the boost's job).
    rows[0].play_count = Some(0);
    rows[1].play_count = Some(60);
    rebuild(&pool, &rows).await.unwrap();
    let hits = search(&pool, "hello", 10).await.unwrap();
    assert_eq!(hits[0].id, "weak");
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
        assert_eq!(hits[0].id, "1");
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
            let rows: Vec<SongSearchRow> =
                sqlx::query_as("SELECT id, title, artist, album, duration, play_count FROM songs")
                    .fetch_all(&pool)
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
