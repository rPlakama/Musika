use super::common::*;
use super::memory_pool;

fn dbs(library: &SqlitePool, media: &SqlitePool) -> Dbs {
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

/// Cover file read + decode, split into stages so an io_uring reader can
/// be compared against `std::fs` without changing the decode harness.
///
/// `cargo test test::benchmark::bench_cover_load -- --ignored --nocapture`
#[ignore = "benchmark, run explicitly"]
#[test]
fn bench_cover_load() {
    use image::{ImageFormat, Rgb, RgbImage};

    const ROUNDS: usize = 40;
    const WARMUP: usize = 3;

    fn percentile(sorted: &[Duration], p: f64) -> Duration {
        let index = ((sorted.len() - 1) as f64 * p).round() as usize;
        sorted[index]
    }

    type Reader = fn(&Path) -> std::io::Result<Vec<u8>>;

    fn sample(path: &Path, reader: Reader) -> (Duration, Duration) {
        let read_started = Instant::now();
        let bytes = reader(path).expect("read cover");
        let read = read_started.elapsed();
        let decode_started = Instant::now();
        let image =
            image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).expect("decode cover");
        let decode = decode_started.elapsed();
        drop(image);
        (read, decode)
    }

    fn read_std(path: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    let dir = std::env::temp_dir().join(format!(
        "musika-cover-bench-{}-{}",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("bench dir");
    let raw_path = dir.join("raw.jpg");
    let thumb_path = dir.join("thumb.jpg");

    let raw = RgbImage::from_fn(2_400, 2_400, |x, y| {
        Rgb([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8])
    });
    image::DynamicImage::ImageRgb8(raw)
        .save_with_format(&raw_path, ImageFormat::Jpeg)
        .expect("save raw");
    let source = image::open(&raw_path).expect("open raw");
    let thumb = source.thumbnail(320, 320);
    thumb
        .save_with_format(&thumb_path, ImageFormat::Jpeg)
        .expect("save thumb");

    #[expect(
        unused_mut,
        reason = "the binding is mutated only under the io-uring feature, which is off by default"
    )]
    let mut readers: Vec<(&str, Reader)> = vec![("std", read_std)];
    #[cfg(feature = "io-uring")]
    readers.push(("io_uring", crate::io_uring_read::read_file));
    for path in [&thumb_path, &raw_path] {
        let name = path.file_stem().unwrap().to_string_lossy();
        let bytes = std::fs::metadata(path).unwrap().len();
        for &(reader_name, reader) in &readers {
            for _ in 0..WARMUP {
                _ = sample(path, reader);
            }
            let mut reads = Vec::with_capacity(ROUNDS);
            let mut decodes = Vec::with_capacity(ROUNDS);
            let started = Instant::now();
            for _ in 0..ROUNDS {
                let (read, decode) = sample(path, reader);
                reads.push(read);
                decodes.push(decode);
            }
            let total = started.elapsed();
            reads.sort();
            decodes.sort();
            println!(
                "[bench] cover.{name}.{reader_name} ({bytes} B): read p50 {:.2} ms p95 {:.2} ms | \
                     decode p50 {:.2} ms p95 {:.2} ms | {:.2} ms/load",
                percentile(&reads, 0.50).as_secs_f64() * 1000.0,
                percentile(&reads, 0.95).as_secs_f64() * 1000.0,
                percentile(&decodes, 0.50).as_secs_f64() * 1000.0,
                percentile(&decodes, 0.95).as_secs_f64() * 1000.0,
                total.as_secs_f64() * 1000.0 / ROUNDS as f64,
            );
        }
    }

    #[cfg(feature = "io-uring")]
    {
        use crate::io_uring_read::Reader;
        let batch: Vec<PathBuf> = (0..32)
            .map(|index| dir.join(format!("batch-{index}.jpg")))
            .collect();
        for path in &batch {
            std::fs::copy(&thumb_path, path).expect("batch cover");
        }
        let reader = Reader::new().expect("batch reader");
        _ = reader.read_many(&batch);
        let started = Instant::now();
        for _ in 0..10 {
            let results = reader.read_many(&batch);
            assert_eq!(results.len(), batch.len());
            assert!(results.iter().all(|result| result.is_ok()));
        }
        println!(
            "[bench] cover.batch.32 io_uring: {:.2} ms/read",
            started.elapsed().as_secs_f64() * 1000.0 / (10 * batch.len()) as f64
        );
    }

    _ = std::fs::remove_dir_all(dir);
}

#[cfg(feature = "io-uring")]
#[test]
fn io_uring_read_many_matches_std_fs() {
    let dir = std::env::temp_dir().join(format!(
        "musika-uring-batch-{}-{}",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let paths: Vec<PathBuf> = (0..5)
        .map(|index| dir.join(format!("file-{index}.bin")))
        .collect();
    for (index, path) in paths.iter().enumerate() {
        let bytes: Vec<u8> = (0..(index * 1000))
            .map(|value| (value % 251) as u8)
            .collect();
        std::fs::write(path, &bytes).expect("write");
    }
    let reader = crate::io_uring_read::Reader::new().expect("reader");
    let results = reader.read_many(&paths);
    for (index, result) in results.into_iter().enumerate() {
        let expected: Vec<u8> = (0..(index * 1000))
            .map(|value| (value % 251) as u8)
            .collect();
        assert_eq!(result.expect("batch read"), expected, "file {index}");
    }
    _ = std::fs::remove_dir_all(dir);
}

#[cfg(feature = "io-uring")]
#[test]
fn io_uring_read_matches_std_fs() {
    let path = std::env::temp_dir().join(format!(
        "musika-uring-read-{}-{}",
        std::process::id(),
        crate::consts::now_nanos()
    ));
    let expected: Vec<u8> = (0..16_384).map(|index| (index % 251) as u8).collect();
    std::fs::write(&path, &expected).expect("write probe");
    let actual = crate::io_uring_read::read_file(&path).expect("io_uring read");
    assert_eq!(actual, expected);
    _ = std::fs::remove_file(path);
}

/// Reports the same GPU/Vulkan facts as the debug binary.
///
/// `cargo test test::benchmark::vulkan_probe -- --nocapture`
#[test]
fn idle_log_labels_are_stable() {
    assert_eq!(crate::debug_info::idle_event(true), "enter");
    assert_eq!(crate::debug_info::idle_event(false), "leave");
}

#[test]
fn vulkan_probe() {
    for line in crate::debug_info::diagnostics() {
        println!("[bench] {line}");
    }
}

/// Compression audit: what gzip saves on the payloads Musika moves or stores.
///
/// The UI cover cache is image data (JPEG/PNG), so gzip there buys little and
/// costs an inflate before every decode; JSON and text payloads are the cases
/// where an HTTP content encoding actually pays off. This benchmark prefers the
/// real cover cache and `index.txt` mirror when present (so the numbers are
/// representative photos, not synthetic noise) and falls back to generated
/// fixtures on a machine without them.
///
/// `cargo test test::benchmark::bench_compression -- --ignored --nocapture`
#[ignore = "benchmark, run explicitly"]
#[test]
fn bench_compression() {
    use flate2::read::GzDecoder;
    use flate2::write::GzEncoder;
    use std::io::{Cursor, Read, Write};

    fn compress_gzip(input: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(input).expect("gzip");
        encoder.finish().expect("finish gzip")
    }

    fn report(name: &str, raw: &[u8]) {
        let started = Instant::now();
        let compressed = compress_gzip(raw);
        let encode = started.elapsed();
        let started = Instant::now();
        let mut decoded = Vec::with_capacity(raw.len());
        GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut decoded)
            .expect("gunzip");
        let decode = started.elapsed();
        assert_eq!(decoded, raw, "gzip round-trip for {name}");
        let saving = (raw.len() as f64 - compressed.len() as f64) * 100.0 / raw.len() as f64;
        println!(
            "[bench] compression.{name}: raw={} B gzip={} B ({saving:+.1}%) enc={:.2} ms dec={:.2} ms",
            raw.len(),
            compressed.len(),
            encode.as_secs_f64() * 1000.0,
            decode.as_secs_f64() * 1000.0,
        );
    }

    /// The largest cached originals of one extension: real provider art is a
    /// better gzip sample than a generated pattern, which can compress well by
    /// accident. Returns `(label, bytes)` tuples, empty when the cache is absent.
    fn cached_originals(extension: &str, limit: usize) -> Vec<(String, Vec<u8>)> {
        let Ok(dir) = crate::db::covers_dir() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut files: Vec<(u64, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?.to_string();
                if !name.starts_with("raw-") || !name.ends_with(&format!(".{extension}")) {
                    return None;
                }
                let size = entry.metadata().ok()?.len();
                Some((size, path))
            })
            .collect();
        files.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
        files
            .into_iter()
            .take(limit)
            .enumerate()
            .filter_map(|(index, (_, path))| {
                let bytes = std::fs::read(&path).ok()?;
                Some((format!("real_{extension}_sample_{}", index + 1), bytes))
            })
            .collect()
    }

    fn encode_image(image: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, format)
            .expect("encode benchmark image");
        bytes.into_inner()
    }

    let jpeg_samples = cached_originals("jpg", 3);
    let png_samples = cached_originals("png", 2);
    if jpeg_samples.is_empty() {
        let cover = image::RgbImage::from_fn(1024, 1024, |x, y| {
            image::Rgb([
                (x.wrapping_mul(31) ^ y) as u8,
                (y.wrapping_mul(17) ^ (x / 3)) as u8,
                ((x + y) % 251) as u8,
            ])
        });
        report(
            "jpeg_synthetic_1024",
            &encode_image(&image::DynamicImage::ImageRgb8(cover), image::ImageFormat::Jpeg),
        );
    }
    for (label, bytes) in &jpeg_samples {
        report(label, bytes);
    }
    for (label, bytes) in &png_samples {
        report(label, bytes);
    }

    // A page/window JSON payload shaped like the bridge payloads (50 rows).
    let rows: Vec<Value> = (0..50)
        .map(|index| {
            json!({
                "id": format!("song-{index:04}"),
                "title": format!("Track title number {index}"),
                "album": "A reasonably long album title for realism",
                "artist": "The Artist Name",
                "subtitle": "12 tracks · 2024",
                "meta": "4:21",
                "song_id": format!("song-{index:04}"),
                "album_id": format!("album-{:02}", index / 10),
                "kind": "song",
                "starred": index % 7 == 0,
            })
        })
        .collect();
    report(
        "json_page_payload",
        &serde_json::to_vec(&rows).expect("serialize page fixture"),
    );

    // The write-only text mirror, real when present (it is a good proxy for
    // any large tab-separated text the app could gzip).
    let mirror = crate::db::mirror_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_else(|| {
            (0..5000)
                .map(|index| {
                    format!(
                        "The Artist Name\tThe Album Title {}\tTrack title number {index}\tsong-{index:04}\n",
                        index / 12
                    )
                })
                .collect::<String>()
                .into_bytes()
        });
    report("text_index_mirror", &mirror);
}
