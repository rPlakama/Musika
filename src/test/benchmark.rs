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
            &encode_image(
                &image::DynamicImage::ImageRgb8(cover),
                image::ImageFormat::Jpeg,
            ),
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

/// Real-corpus cover pipeline: the current display-sized decode split into its
/// stages, compared with the pre-display-cap path (a 320 px thumbnail painted
/// immediately, then a generated 1024 px display copy decoded for the hero).
///
/// The corpus is read-only. `MUSIKA_BENCH_COVERS` overrides the directory; it
/// defaults to the sandbox cache when that exists and prints a skip otherwise.
///
/// `cargo test --release test::benchmark::bench_cover_pipeline -- --ignored --nocapture`
#[cfg(feature = "gpuui")]
#[ignore = "benchmark, run explicitly"]
#[test]
fn bench_cover_pipeline() {
    cover_pipeline::run();
}

/// Helpers for [`bench_cover_pipeline`], gated with the UI feature they
/// measure.
#[cfg(feature = "gpuui")]
mod cover_pipeline {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Originals decoded per run. Full decode is the cost under test, so a
    /// size-stratified sample keeps the run near a minute instead of decoding
    /// all 829 originals.
    const SAMPLE_TARGET: usize = 40;
    /// Minimum sampled files per long-edge cohort: the size strata can miss the
    /// 500 px originals, which are not the smallest files.
    const COHORT_MIN: usize = 3;
    /// Repetitions per file and stage; the cohort table pools them.
    const REPS: usize = 5;
    /// Hero edge for the 505x909 window named by the page-load task.
    const HERO_EDGE: u32 = 485;
    /// Pause between timed iterations, paired with `fadvise(DONTNEED)`.
    const COOLDOWN_MS: u64 = 5;
    /// The migration gate mirrored by the optional startup probe.
    const COVER_MAX_EDGE: u32 = 4096;
    /// JPEG quality `covers::cap_oversized_art` uses when it re-encodes.
    const CAP_QUALITY: u8 = 92;

    const METRIC_COUNT: usize = 9;
    const M_READ: usize = 0;
    const M_DECODE: usize = 1;
    const M_THUMB: usize = 2;
    const M_BGRA: usize = 3;
    const M_P64: usize = 4;
    const M_P485: usize = 5;
    const M_P900: usize = 6;
    const M_OLD1024: usize = 7;
    const M_OLD320: usize = 8;

    const COHORT_LABELS: [&str; 4] = ["<600", "600-1500", "1500-3000", ">3000"];

    type Timings = [Vec<Duration>; METRIC_COUNT];

    struct Original {
        path: PathBuf,
        stem: String,
        bytes: u64,
        edge: u32,
        cohort: usize,
    }

    struct Sample {
        path: PathBuf,
        bytes: u64,
        edge: u32,
        cohort: usize,
        thumb: Option<PathBuf>,
        display: Option<PathBuf>,
    }

    #[derive(Default)]
    struct Stats {
        n: usize,
        p50: f64,
        p95: f64,
    }

    fn empty_timings() -> Timings {
        std::array::from_fn(|_| Vec::new())
    }

    fn millis(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1000.0
    }

    fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
        let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
        sorted[index]
    }

    fn stats(values: &[Duration]) -> Stats {
        if values.is_empty() {
            return Stats::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        Stats {
            n: values.len(),
            p50: millis(percentile(&sorted, 0.50)),
            p95: millis(percentile(&sorted, 0.95)),
        }
    }

    /// Percentiles over per-file values with corpus-frequency weights, so a
    /// deliberately oversampled tail does not become the "typical" page.
    fn weighted_stats(values: &[(f64, f64)]) -> Stats {
        if values.is_empty() {
            return Stats::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
        let total: f64 = sorted.iter().map(|(_, weight)| *weight).sum();
        if total <= 0.0 {
            return Stats::default();
        }
        let pick = |fraction: f64| -> f64 {
            let target = total * fraction;
            let mut cumulative = 0.0;
            for (value, weight) in &sorted {
                cumulative += *weight;
                if cumulative >= target {
                    return *value;
                }
            }
            sorted[sorted.len() - 1].0
        };
        Stats {
            n: values.len(),
            p50: pick(0.50),
            p95: pick(0.95),
        }
    }

    fn cell(stats: &Stats) -> String {
        if stats.n == 0 {
            "-".to_string()
        } else {
            format!("{:.2}", stats.p50)
        }
    }

    fn cell95(stats: &Stats) -> String {
        if stats.n == 0 {
            "-".to_string()
        } else {
            format!("{:.2}", stats.p95)
        }
    }

    /// Evicts the file's clean page-cache pages (best effort) and pauses, so a
    /// timed read measures the file rather than a hot cache. Decode dominates,
    /// but the read stage should not look free on a re-read.
    fn cool_down(path: &Path) {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd as _;
            if let Ok(file) = std::fs::File::open(path) {
                // SAFETY: `posix_fadvise` only inspects the open file
                // descriptor; `file` keeps that descriptor alive for the call
                // and no pointer crosses the boundary.
                _ = unsafe {
                    libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED)
                };
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = path;
        std::thread::sleep(Duration::from_millis(COOLDOWN_MS));
    }

    fn cohort_of(edge: u32) -> usize {
        match edge {
            0..=599 => 0,
            600..=1499 => 1,
            1500..=2999 => 2,
            _ => 3,
        }
    }

    fn sibling(dir: &Path, stem: &str, suffix: &str) -> Option<PathBuf> {
        for extension in ["jpg", "png"] {
            let candidate = dir.join(format!("{stem}{suffix}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }

    fn scan_originals(dir: &Path) -> Vec<Original> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut originals = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with("raw-") || name.contains(".thumb.") || name.contains("_1024x1024")
            {
                continue;
            }
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                continue;
            };
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp"
            ) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let stem = stem.to_string();
            originals.push(Original {
                path,
                stem,
                bytes: metadata.len(),
                edge: 0,
                cohort: 0,
            });
        }
        originals.sort_unstable_by_key(|original| original.bytes);
        originals
    }

    /// 12 largest originals, 12 around the median, and 16 evenly spaced over
    /// the remaining size ranks; then every long-edge cohort is topped up to
    /// [`COHORT_MIN`] files so the tiny-original row is not empty.
    fn choose_sample(dir: &Path, originals: &[Original]) -> Vec<Sample> {
        let count = originals.len();
        let mut picked = HashSet::new();
        for index in count.saturating_sub(12)..count {
            picked.insert(index);
        }
        let median = count / 2;
        for index in median.saturating_sub(6)..(median + 6).min(count) {
            picked.insert(index);
        }
        let rest: Vec<usize> = (0..count).filter(|index| !picked.contains(index)).collect();
        let missing = SAMPLE_TARGET.saturating_sub(picked.len());
        if missing > 0 && !rest.is_empty() {
            let last = rest.len() - 1;
            let divisor = missing.saturating_sub(1).max(1);
            for step in 0..missing {
                picked.insert(rest[step * last / divisor]);
            }
        }
        for cohort in 0..4 {
            let sampled = picked
                .iter()
                .filter(|index| originals[**index].cohort == cohort)
                .count();
            if sampled >= COHORT_MIN {
                continue;
            }
            let candidates: Vec<usize> = (0..count)
                .filter(|index| originals[*index].cohort == cohort && !picked.contains(index))
                .collect();
            let wanted = COHORT_MIN.saturating_sub(sampled);
            if candidates.is_empty() {
                continue;
            }
            let last = candidates.len() - 1;
            let divisor = wanted.saturating_sub(1).max(1);
            for step in 0..wanted {
                picked.insert(candidates[step * last / divisor]);
            }
        }
        let mut indices: Vec<usize> = picked.into_iter().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .map(|index| {
                let original = &originals[index];
                Sample {
                    path: original.path.clone(),
                    bytes: original.bytes,
                    edge: original.edge,
                    cohort: original.cohort,
                    thumb: sibling(dir, &original.stem, ".thumb"),
                    display: sibling(dir, &original.stem, "_1024x1024"),
                }
            })
            .collect()
    }

    /// The pre-cap UI loader: full decode at the file's own size, then the
    /// RGBA -> BGRA swap gpui's stock decoder performs. No thumbnail step.
    fn decode_stock(path: &Path) -> Result<(), image::ImageError> {
        let bytes = std::fs::read(path).map_err(image::ImageError::IoError)?;
        let format = image::guess_format(&bytes)?;
        let mut rgba = image::load_from_memory_with_format(&bytes, format)?.into_rgba8();
        for pixel in rgba.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        std::hint::black_box(&rgba);
        Ok(())
    }

    fn measure_stock(path: &Path, out: &mut Vec<Duration>) {
        _ = decode_stock(path);
        for _ in 0..REPS {
            cool_down(path);
            let started = Instant::now();
            let decoded = decode_stock(path);
            let elapsed = started.elapsed();
            if decoded.is_ok() {
                out.push(elapsed);
            }
        }
    }

    /// Bench-local copy of the migration's re-encode step: read, full decode,
    /// thumbnail to the cap, JPEG encode. The production gate first probes
    /// dimensions and skips anything already below [`COVER_MAX_EDGE`].
    fn cap_equivalent(path: &Path) -> Result<usize, image::ImageError> {
        let bytes = std::fs::read(path).map_err(image::ImageError::IoError)?;
        let format = image::guess_format(&bytes)?;
        let decoded = image::load_from_memory_with_format(&bytes, format)?;
        let resized = decoded.thumbnail(COVER_MAX_EDGE, COVER_MAX_EDGE);
        let mut output = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, CAP_QUALITY)
            .encode_image(&resized.to_rgb8())?;
        Ok(output.into_inner().len())
    }

    pub(super) fn run() {
        let dir = std::env::var("MUSIKA_BENCH_COVERS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/musika-mem/config-src/covers"));
        if !dir.is_dir() {
            println!(
                "[bench] cover.pipeline: skipped, corpus {} is not a directory",
                dir.display()
            );
            return;
        }
        let mut originals = scan_originals(&dir);
        if originals.is_empty() {
            println!(
                "[bench] cover.pipeline: skipped, no raw-* originals in {}",
                dir.display()
            );
            return;
        }

        // One header probe per cached original, the same scan the startup
        // migration does. The measured longest edges also drive the cohorts.
        let probe_started = Instant::now();
        let mut probe_times = Vec::with_capacity(originals.len());
        let mut oversized = 0usize;
        for original in &mut originals {
            let started = Instant::now();
            if let Ok((width, height)) = image::image_dimensions(&original.path) {
                probe_times.push(started.elapsed());
                original.edge = width.max(height);
                original.cohort = cohort_of(original.edge);
                if original.edge > COVER_MAX_EDGE {
                    oversized += 1;
                }
            }
        }
        let probe_total = probe_started.elapsed();
        let mut corpus_counts = [0usize; 4];
        for original in &originals {
            corpus_counts[original.cohort] += 1;
        }

        let sample = choose_sample(&dir, &originals);
        if sample.is_empty() {
            println!("[bench] cover.pipeline: skipped, sample is empty");
            return;
        }

        let mut cohort_counts = [0usize; 4];
        for file in &sample {
            cohort_counts[file.cohort] += 1;
        }
        let mut sizes: Vec<u64> = sample.iter().map(|file| file.bytes).collect();
        sizes.sort_unstable();
        let total_mib = sizes.iter().sum::<u64>() as f64 / (1024.0 * 1024.0);
        let median_mib = sizes[sizes.len() / 2] as f64 / (1024.0 * 1024.0);
        println!(
            "[bench] cover.pipeline corpus={} originals={} sampled={} reps={} hero_edge={} old_edge=1024 cooldown={}ms",
            dir.display(),
            originals.len(),
            sample.len(),
            REPS,
            HERO_EDGE,
            COOLDOWN_MS
        );
        println!(
            "[bench] cover.pipeline sample total={total_mib:.1} MiB median={median_mib:.2} MiB cohorts <600:{} 600-1500:{} 1500-3000:{} >=3000:{}",
            cohort_counts[0], cohort_counts[1], cohort_counts[2], cohort_counts[3]
        );
        let corpus_total = originals.len().max(1) as f64;
        println!(
            "[bench] cover.pipeline corpus cohorts <600:{} ({:.1}%) 600-1500:{} ({:.1}%) 1500-3000:{} ({:.1}%) >=3000:{} ({:.1}%) of {} originals",
            corpus_counts[0],
            corpus_counts[0] as f64 / corpus_total * 100.0,
            corpus_counts[1],
            corpus_counts[1] as f64 / corpus_total * 100.0,
            corpus_counts[2],
            corpus_counts[2] as f64 / corpus_total * 100.0,
            corpus_counts[3],
            corpus_counts[3] as f64 / corpus_total * 100.0,
            originals.len()
        );
        println!(
            "[bench] cover.pipeline reads use posix_fadvise(DONTNEED); stage rows are one pass (read, guess+full decode, thumbnail({HERO_EDGE}), RGBA->BGRA)"
        );

        let derivative_dir = std::env::temp_dir().join(format!(
            "musika-bench-derivatives-{}-{}",
            std::process::id(),
            crate::consts::now_nanos()
        ));
        _ = std::fs::create_dir_all(&derivative_dir);
        let mut derivative_warm: Vec<(f64, usize)> = Vec::with_capacity(sample.len());

        let mut cohort_timings: [Timings; 4] = std::array::from_fn(|_| empty_timings());
        let mut all_timings = empty_timings();
        let mut hero_per_file: Vec<(f64, usize)> = Vec::with_capacity(sample.len());
        let mut thumb_per_file: Vec<(f64, usize)> = Vec::with_capacity(sample.len());
        let mut slowest: Vec<(f64, u32, String)> = Vec::with_capacity(sample.len());

        for file in &sample {
            let mut timings = empty_timings();

            // One untimed pass primes the codec and the allocator.
            if let Ok(bytes) = std::fs::read(&file.path)
                && let Ok(format) = image::guess_format(&bytes)
                && let Ok(decoded) = image::load_from_memory_with_format(&bytes, format)
            {
                let mut rgba = decoded.thumbnail(HERO_EDGE, HERO_EDGE).into_rgba8();
                for pixel in rgba.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                std::hint::black_box(&rgba);
            }

            for _ in 0..REPS {
                cool_down(&file.path);
                let started = Instant::now();
                let Ok(bytes) = std::fs::read(&file.path) else {
                    continue;
                };
                timings[M_READ].push(started.elapsed());

                let started = Instant::now();
                let decoded = image::guess_format(&bytes)
                    .and_then(|format| image::load_from_memory_with_format(&bytes, format));
                let decode = started.elapsed();
                let Ok(decoded) = decoded else {
                    continue;
                };
                timings[M_DECODE].push(decode);

                let started = Instant::now();
                let thumbed = decoded.thumbnail(HERO_EDGE, HERO_EDGE);
                timings[M_THUMB].push(started.elapsed());

                let started = Instant::now();
                let mut rgba = thumbed.into_rgba8();
                for pixel in rgba.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                timings[M_BGRA].push(started.elapsed());
                std::hint::black_box(&rgba);
            }

            for (metric, edge) in [(M_P64, 64), (M_P485, HERO_EDGE), (M_P900, 900)] {
                for _ in 0..REPS {
                    cool_down(&file.path);
                    let started = Instant::now();
                    // Cold production path with the display-derivative cache
                    // disabled: this is the first-ever open of the cover.
                    let decoded =
                        crate::ui::fast_image::decode_for_display_in(&file.path, edge, None);
                    let elapsed = started.elapsed();
                    if decoded.is_ok() {
                        timings[metric].push(elapsed);
                    }
                }
            }

            // Second open: the bucket-sized derivative is on disk now, so the
            // hero decode is a small JPEG read instead of the original.
            for _ in 0..2 {
                _ = crate::ui::fast_image::decode_for_display_in(
                    &file.path,
                    HERO_EDGE,
                    Some(&derivative_dir),
                );
            }
            let started = Instant::now();
            let warm = crate::ui::fast_image::decode_for_display_in(
                &file.path,
                HERO_EDGE,
                Some(&derivative_dir),
            );
            if warm.is_ok() {
                derivative_warm.push((millis(started.elapsed()), file.cohort));
            }

            if let Some(thumb) = &file.thumb {
                measure_stock(thumb, &mut timings[M_OLD320]);
            }
            if let Some(display) = &file.display {
                measure_stock(display, &mut timings[M_OLD1024]);
            }

            for (metric, values) in timings.iter().enumerate() {
                cohort_timings[file.cohort][metric].extend_from_slice(values);
                all_timings[metric].extend_from_slice(values);
            }
            let hero = stats(&timings[M_P485]);
            hero_per_file.push((hero.p50, file.cohort));
            let thumb = stats(&timings[M_OLD320]);
            thumb_per_file.push((thumb.p50, file.cohort));
            slowest.push((
                hero.p50,
                file.edge,
                file.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("?")
                    .to_string(),
            ));
        }

        println!();
        println!(
            "[bench] pipeline.table (ms; cohort p50 unless .95 suffix; n = files, samples = n*{REPS})"
        );
        println!(
            "[bench] pipeline {:<10} {:>3}  {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>7} {:>8} {:>7}",
            "cohort",
            "n",
            "read",
            "decod",
            "thumb",
            "bgra",
            "p64",
            "p485",
            "p485.95",
            "p900",
            "old1024",
            "old320"
        );
        for (cohort, label) in COHORT_LABELS.iter().enumerate() {
            let files = cohort_counts[cohort];
            if files == 0 {
                continue;
            }
            let row: [Stats; METRIC_COUNT] =
                std::array::from_fn(|metric| stats(&cohort_timings[cohort][metric]));
            println!(
                "[bench] pipeline {:<10} {:>3}  {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>7} {:>8} {:>7}",
                label,
                files,
                cell(&row[M_READ]),
                cell(&row[M_DECODE]),
                cell(&row[M_THUMB]),
                cell(&row[M_BGRA]),
                cell(&row[M_P64]),
                cell(&row[M_P485]),
                cell95(&row[M_P485]),
                cell(&row[M_P900]),
                cell(&row[M_OLD1024]),
                cell(&row[M_OLD320])
            );
        }
        let all: [Stats; METRIC_COUNT] = std::array::from_fn(|metric| stats(&all_timings[metric]));
        println!(
            "[bench] pipeline {:<10} {:>3}  {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>7} {:>8} {:>7}",
            "all",
            sample.len(),
            cell(&all[M_READ]),
            cell(&all[M_DECODE]),
            cell(&all[M_THUMB]),
            cell(&all[M_BGRA]),
            cell(&all[M_P64]),
            cell(&all[M_P485]),
            cell95(&all[M_P485]),
            cell(&all[M_P900]),
            cell(&all[M_OLD1024]),
            cell(&all[M_OLD320])
        );
        println!(
            "[bench] pipeline note: old1024 in the table covers only sampled files that still have a leftover display copy (n={}); a stable old-hero figure over every leftover copy follows)",
            sample.iter().filter(|file| file.display.is_some()).count()
        );
        slowest.sort_unstable_by(|left, right| right.0.total_cmp(&left.0));
        for (p50, edge, name) in slowest.iter().take(3) {
            println!("[bench] pipeline.slowest p485 {name} edge={edge}px p50={p50:.2} ms");
        }

        // The old hero source on this corpus: every leftover `_1024x1024`
        // display copy, independent of the stratified sample.
        let mut leftover_timings = Vec::new();
        let mut leftover_files = 0usize;
        let mut leftover_bytes = 0u64;
        for original in &originals {
            let Some(display) = sibling(&dir, &original.stem, "_1024x1024") else {
                continue;
            };
            leftover_files += 1;
            leftover_bytes = leftover_bytes
                .saturating_add(std::fs::metadata(&display).map_or(0, |meta| meta.len()));
            measure_stock(&display, &mut leftover_timings);
        }
        let old_hero = stats(&leftover_timings);
        let new_hero = stats(&all_timings[M_P485]);
        let old_thumb = stats(&all_timings[M_OLD320]);
        println!(
            "[bench] old.hero.1024 leftover display copies n={leftover_files} avg={:.0} KiB p50={:.2} ms p95={:.2} ms",
            leftover_bytes as f64 / (1024.0 * leftover_files.max(1) as f64),
            old_hero.p50,
            old_hero.p95
        );

        if !derivative_warm.is_empty() {
            let warm_all: Vec<Duration> = derivative_warm
                .iter()
                .map(|(ms, _)| Duration::from_secs_f64(ms / 1000.0))
                .collect();
            let warm = stats(&warm_all);
            println!(
                "[bench] derivative.warm hero={HERO_EDGE} second-open n={} p50={:.2} p95={:.2} ms (bucket-sized JPEG decode, no original decode)",
                warm.n, warm.p50, warm.p95
            );
            for (cohort, label) in COHORT_LABELS.iter().enumerate() {
                let values: Vec<Duration> = derivative_warm
                    .iter()
                    .filter(|(_, index)| *index == cohort)
                    .map(|(ms, _)| Duration::from_secs_f64(ms / 1000.0))
                    .collect();
                let stats = stats(&values);
                if stats.n > 0 {
                    println!(
                        "[bench] derivative.warm cohort {label:<9} n={:<3} p50={:.2} p95={:.2} ms",
                        stats.n, stats.p50, stats.p95
                    );
                }
            }
            _ = std::fs::remove_dir_all(&derivative_dir);
        }

        let new_64 = stats(&all_timings[M_P64]);
        let new_900 = stats(&all_timings[M_P900]);
        let mut weighted_hero = Vec::with_capacity(hero_per_file.len());
        for (value, cohort) in &hero_per_file {
            weighted_hero.push((
                *value,
                corpus_counts[*cohort] as f64 / cohort_counts[*cohort].max(1) as f64,
            ));
        }
        let mut weighted_thumb = Vec::with_capacity(thumb_per_file.len());
        for (value, cohort) in &thumb_per_file {
            weighted_thumb.push((
                *value,
                corpus_counts[*cohort] as f64 / cohort_counts[*cohort].max(1) as f64,
            ));
        }
        let weighted_new_hero = weighted_stats(&weighted_hero);
        let weighted_old_thumb = weighted_stats(&weighted_thumb);
        println!();
        println!("[bench] page-open estimate (p50 / p95, ms)");
        println!(
            "[bench] page-open new.hero485        {:.2} / {:.2}  (n={})",
            new_hero.p50, new_hero.p95, new_hero.n
        );
        println!(
            "[bench] page-open old.hero1024       {:.2} / {:.2}  (n={} leftover display copies)",
            old_hero.p50, old_hero.p95, old_hero.n
        );
        println!(
            "[bench] page-open old.first_paint320 {:.2} / {:.2}  (n={})",
            old_thumb.p50, old_thumb.p95, old_thumb.n
        );
        println!(
            "[bench] page-open new.row64          {:.2} / {:.2}",
            new_64.p50, new_64.p95
        );
        println!(
            "[bench] page-open new.fullscreen900  {:.2} / {:.2}",
            new_900.p50, new_900.p95
        );
        if old_hero.p50 > 0.0 {
            let ratio = new_hero.p50 / old_hero.p50;
            let sequential = old_hero.p50 + old_thumb.p50;
            println!(
                "[bench] page-open ratio new485/old1024 = {ratio:.2}x | new485/(old1024+old320) = {:.2}x (higher = current path is slower)",
                new_hero.p50 / sequential
            );
            println!(
                "[bench] page-open weighted new.hero485 {:.2} / {:.2} | weighted old.thumb320 {:.2} / {:.2}",
                weighted_new_hero.p50,
                weighted_new_hero.p95,
                weighted_old_thumb.p50,
                weighted_old_thumb.p95
            );
            println!(
                "[bench] page-open weighted ratio (corpus cohort frequencies) new485/old1024 = {:.2}x | new485/(old1024+old320) = {:.2}x",
                weighted_new_hero.p50 / old_hero.p50,
                weighted_new_hero.p50 / (old_hero.p50 + weighted_old_thumb.p50)
            );
        }
        if new_hero.p50 > 0.0 {
            println!(
                "[bench] stage.share.of.e2e485 read={:.1}% decode={:.1}% thumbnail={:.1}% bgra={:.1}% (stages measured as separate passes, so the sum can exceed the e2e p50)",
                all[M_READ].p50 / new_hero.p50 * 100.0,
                all[M_DECODE].p50 / new_hero.p50 * 100.0,
                all[M_THUMB].p50 / new_hero.p50 * 100.0,
                all[M_BGRA].p50 / new_hero.p50 * 100.0
            );
        }

        // Optional startup migration probe: the dimension scan ran before the
        // sample above; the re-encode timings below are for the largest five.
        println!();
        let probe = stats(&probe_times);
        println!(
            "[bench] startup.migration.dimensions n={} total={:.2} ms p50={:.3} ms p95={:.3} ms >{COVER_MAX_EDGE}px={oversized}",
            probe.n,
            millis(probe_total),
            probe.p50,
            probe.p95
        );
        println!(
            "[bench] startup.migration.cap largest 5 (read+full decode+thumbnail({COVER_MAX_EDGE})+JPEG q{CAP_QUALITY}); the real gate skips files <= {COVER_MAX_EDGE} px, so only {oversized} qualify"
        );
        let mut largest: Vec<&Original> = originals.iter().collect();
        largest.sort_unstable_by_key(|original| std::cmp::Reverse(original.bytes));
        for original in largest.into_iter().take(5) {
            let started = Instant::now();
            match cap_equivalent(&original.path) {
                Ok(out_bytes) => println!(
                    "[bench] startup.migration.cap {} {:.2} MiB out={:.0} KiB total={:.2} ms",
                    original
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("?"),
                    original.bytes as f64 / (1024.0 * 1024.0),
                    out_bytes as f64 / 1024.0,
                    millis(started.elapsed())
                ),
                Err(_) => println!(
                    "[bench] startup.migration.cap {} failed after {:.2} ms",
                    original.path.display(),
                    millis(started.elapsed())
                ),
            }
        }
    }
}

/// Local radio expansion against a real `library.db` copy.
///
/// `MUSIKA_RADIO_DB` overrides the database path (default: the sandbox copy of
/// the real library). Read-only: the benchmark never writes to the database.
///
/// `cargo test --lib test::benchmark::bench_local_radio -- --ignored --nocapture`
#[ignore = "benchmark, run explicitly"]
#[tokio::test]
async fn bench_local_radio() {
    let path = std::env::var("MUSIKA_RADIO_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/musika-mem/config-src/library.db"));
    if !path.is_file() {
        println!(
            "[bench] local.radio: skipped, {} is not a file",
            path.display()
        );
        return;
    }
    // The corpus snapshot predates the radio migration: work on a temp copy
    // and run the current migrations so the benchmark exercises the shipping
    // schema. The source snapshot is never written.
    let temp = std::env::temp_dir().join(format!("musika-bench-radio-{}.db", std::process::id()));
    std::fs::copy(&path, &temp).expect("copy library for the benchmark");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&temp);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .expect("open library copy");
    sqlx::migrate!("./migrations_library")
        .run(&pool)
        .await
        .expect("migrate benchmark copy");
    let dbs = dbs_on(&pool);

    let songs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM songs")
        .fetch_one(&pool)
        .await
        .expect("song count");
    let seeds: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM songs
         WHERE genre IS NOT NULL AND genre <> '' AND artist_id IS NOT NULL
         ORDER BY RANDOM() LIMIT 40",
    )
    .fetch_all(&pool)
    .await
    .expect("seed ids");
    if seeds.is_empty() {
        println!("[bench] local.radio: skipped, no genre-tagged songs");
        return;
    }

    for seed in seeds.iter().take(10) {
        _ = crate::recommend::similar_songs(&dbs, "song", seed, 100, false, 1)
            .await
            .expect("warmup expansion");
    }
    let mut timings = Vec::with_capacity(seeds.len());
    for (index, seed) in seeds.iter().enumerate() {
        let started = Instant::now();
        let rows =
            crate::recommend::similar_songs(&dbs, "song", seed, 100, false, index as u64 + 1)
                .await
                .expect("radio expansion");
        timings.push(started.elapsed());
        assert!(
            !rows.is_empty(),
            "every seed must produce a non-empty radio"
        );
    }
    timings.sort();
    let percentile = |fraction: f64| {
        let index = ((timings.len() - 1) as f64 * fraction).round() as usize;
        timings[index].as_secs_f64() * 1000.0
    };
    let mean = timings.iter().map(|time| time.as_secs_f64()).sum::<f64>() / timings.len() as f64;
    println!(
        "[bench] local.radio songs={songs} seeds={} count=100: mean={:.2} ms p50={:.2} ms p95={:.2} ms",
        timings.len(),
        mean * 1000.0,
        percentile(0.50),
        percentile(0.95)
    );

    let seed = &seeds[0];
    let info: (String, Option<String>, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT title, artist, genre, year FROM songs WHERE id = ?1")
            .bind(seed)
            .fetch_one(&pool)
            .await
            .expect("seed row");
    println!(
        "[bench] local.radio sample seed: {} — {} · {:?} {}",
        info.1.unwrap_or_default(),
        info.0,
        info.2.unwrap_or_default(),
        info.3.unwrap_or(0)
    );
    let rows = crate::recommend::similar_songs(&dbs, "song", seed, 100, false, 1)
        .await
        .expect("sample expansion");
    println!("[bench] local.radio sample mix:");
    for row in rows.iter().take(10) {
        let extra: Option<(Option<String>, Option<i32>)> =
            sqlx::query_as("SELECT genre, year FROM songs WHERE id = ?1")
                .bind(&row.id)
                .fetch_optional(&pool)
                .await
                .expect("row metadata");
        let (genre, year) = extra.unwrap_or_default();
        println!(
            "[bench]   {} — {} · {:?} {}",
            row.artist.clone().unwrap_or_default(),
            row.title,
            genre.unwrap_or_default(),
            year.unwrap_or(0)
        );
    }
    drop(pool);
    _ = std::fs::remove_file(&temp);
}
