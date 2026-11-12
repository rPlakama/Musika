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

    #[allow(unused_mut)]
    let mut readers: Vec<(&str, Reader)> = vec![("std", read_std)];
    #[cfg(feature = "io-uring")]
    readers.push(("io_uring", crate::io_uring_read::read_file));
    for path in [&thumb_path, &raw_path] {
        let name = path.file_stem().unwrap().to_string_lossy();
        let bytes = std::fs::metadata(path).unwrap().len();
        for &(reader_name, reader) in &readers {
            for _ in 0..WARMUP {
                let _ = sample(path, reader);
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
        let _ = reader.read_many(&batch);
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

    let _ = std::fs::remove_dir_all(dir);
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
    let _ = std::fs::remove_dir_all(dir);
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
    let _ = std::fs::remove_file(path);
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
