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
