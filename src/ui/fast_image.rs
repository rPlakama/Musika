// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Display-sized cover decoding for the GPUI front end.
//!
//! GPUI's stock loader decodes a cover at its stored resolution and uploads
//! that texture; the app's previous loader decoded the untouched provider
//! original even when a surface only paints a few hundred pixels. For a
//! 4000x4000 provider JPEG that is ~64 MB of transient RGBA and 150-400 ms of
//! JPEG work before the first pixel can be painted.
//!
//! This module decodes for the **pixels actually painted**:
//!
//! - JPEGs whose long edge is at least twice the target use `jpeg-decoder`'s
//!   DCT-scaled decode (`1/8`, `1/4`, `1/2`), so the inverse DCT only runs on
//!   the reduced grid, then `fast_image_resize` (AVX2/NEON) resizes the
//!   remainder to the exact bucket edge.
//! - Smaller JPEGs decode directly to BGRA with `zune-jpeg`, which is SIMD
//!   accelerated and writes the channel order GPUI wants, so no per-pixel
//!   swap pass runs for that path.
//! - PNG/WebP/BMP/TIFF still decode through `image`, then resize with
//!   `fast_image_resize` and swap to BGRA in one linear pass.
//! - The decoded display-sized image is written once to a small on-disk
//!   derivative cache (`MUSIKA_UI_COVER_CACHE_DIR`, default under
//!   `$XDG_CACHE_HOME/musika/cover-derivatives`). Later opens of the same
//!   cover read a ~20-80 KB file instead of decoding the original again, and
//!   the derivative decode skips resizing entirely.
//!
//! All of this runs on the background executor (the cache calls it from a
//! spawned task); the UI thread only builds the [`gpui::RenderImage`] handle.

use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{ImageCacheError, RenderImage};
use image::{Frame, ImageEncoder as _};

/// Version tag mixed into derivative file names. Bump when the decoder
/// output changes (filter, colour order, quality) so stale derivatives are
/// not reused.
const DERIVATIVE_VERSION: u32 = 1;
/// JPEG quality for display derivatives. The derivative is already painted
/// at a few hundred pixels; 90 keeps photographic covers visually identical
/// at ~20-80 KB per file.
const DERIVATIVE_QUALITY: u8 = 90;
/// Smallest display edge that is worth a derivative. Row/queue covers decode
/// their 320 px thumbnail directly and never need one.
const MIN_DERIVATIVE_EDGE: u32 = 256;
/// Decode edges are quantized so one decoded image serves a range of window
/// sizes, the page hero and the fullscreen cover. A larger bucket is painted
/// scaled down by the GPU, which is effectively free.
const EDGE_BUCKETS: [u32; 13] = [
    64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096,
];
/// Non-JPEG formats decode through the `image` crate. Animated GIFs and SVG
/// assets deliberately keep GPUI's stock loader and are not listed here.
const RASTER_FORMATS: [image::ImageFormat; 7] = [
    image::ImageFormat::Png,
    image::ImageFormat::WebP,
    image::ImageFormat::Bmp,
    image::ImageFormat::Tiff,
    image::ImageFormat::Ico,
    image::ImageFormat::Qoi,
    image::ImageFormat::Hdr,
];

/// Rounds a requested paint edge up to the next decode bucket.
///
/// Decoding a 505 px hero into a 512 px texture (and a 909 px fullscreen cover
/// into 1024) means a window resize or a surface change reuses the decoded
/// image instead of starting another full decode.
pub(crate) fn quantize_edge(edge: u32) -> u32 {
    EDGE_BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= edge)
        .unwrap_or_else(|| EDGE_BUCKETS.last().copied().unwrap_or(4096))
}

/// Decodes one local cover file for a surface whose longest painted edge is
/// `requested_edge` physical pixels.
///
/// This blocks: the caller runs it on the background executor.
pub(crate) fn decode_for_display(
    path: &Path,
    requested_edge: u32,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    decode_for_display_in(path, requested_edge, derivative_root().as_deref())
}

/// [`decode_for_display`] with an explicit derivative directory. Tests pass a
/// temporary directory (and `None` to disable the on-disk derivative).
pub(crate) fn decode_for_display_in(
    path: &Path,
    requested_edge: u32,
    derivative_dir: Option<&Path>,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let edge = quantize_edge(requested_edge.max(1));
    if edge >= MIN_DERIVATIVE_EDGE
        && let Some(dir) = derivative_dir
        && let Some(derivative) = derivative_path(path, edge, dir)
        && let Ok(image) = decode_derivative(&derivative)
    {
        return Ok(image);
    }
    let bytes = std::fs::read(path).map_err(io_error)?;
    let format = image::guess_format(&bytes).map_err(image_error)?;
    let source_long = source_long_edge(&bytes, format)?;
    let image = if format == image::ImageFormat::Jpeg {
        decode_jpeg(&bytes, edge)?
    } else if RASTER_FORMATS.contains(&format) {
        decode_raster(&bytes, format, edge)?
    } else {
        return Err(message_error(format!(
            "unsupported cover format: {format:?}"
        )));
    };
    // A 1.5x downscale still pays the full source decode, and the
    // bucket-sized JPEG is ~4-6x cheaper to decode on every later open.
    let derivative_worth_it = source_long > edge.saturating_add(edge / 2);
    if edge >= MIN_DERIVATIVE_EDGE
        && derivative_worth_it
        && let Some(dir) = derivative_dir
        && let Err(err) = write_derivative(path, edge, dir, &image)
    {
        // A read-only or full cache directory must never fail the cover: the
        // decoded image is already in memory and correct.
        eprintln!("[musika/cover] derivative write failed: {err}");
    }
    Ok(image)
}

/// Long edge of the stored source, read from headers when possible.
fn source_long_edge(bytes: &[u8], format: image::ImageFormat) -> Result<u32, ImageCacheError> {
    if format == image::ImageFormat::Jpeg {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
        decoder.read_info().map_err(decode_error)?;
        let info = decoder
            .info()
            .ok_or_else(|| message_error("JPEG headers were not read"))?;
        return Ok(u32::from(info.width.max(info.height)));
    }
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(io_error)?;
    let (width, height) = reader.into_dimensions().map_err(image_error)?;
    Ok(width.max(height))
}

/// Decodes a JPEG, using DCT scaling when the source is much larger than the
/// painted edge.
fn decode_jpeg(bytes: &[u8], edge: u32) -> Result<Arc<RenderImage>, ImageCacheError> {
    let mut info_decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    info_decoder.read_info().map_err(decode_error)?;
    let info = info_decoder
        .info()
        .ok_or_else(|| message_error("JPEG headers were not read"))?;
    let (source_w, source_h) = (u32::from(info.width), u32::from(info.height));
    let source_long = source_w.max(source_h);

    // At least 2x downscale: a DCT factor of 1/2, 1/4 or 1/8 exists and the
    // inverse DCT runs on the reduced grid instead of every 8x8 block.
    if source_long > edge.saturating_mul(2) {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
        let (scaled_w, scaled_h) = decoder
            .scale(edge as u16, edge as u16)
            .map_err(decode_error)?;
        let rgb = decoder.decode().map_err(decode_error)?;
        let (target_w, target_h) = fit_dims(u32::from(scaled_w), u32::from(scaled_h), edge);
        let rgb = resize_rgb(
            rgb,
            u32::from(scaled_w),
            u32::from(scaled_h),
            target_w,
            target_h,
        )?;
        return render_bgra(rgb_to_bgra(&rgb), target_w, target_h);
    }

    // Small enough to decode directly: zune-jpeg is SIMD accelerated and can
    // emit BGRA, so no extra swap pass is needed.
    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::BGRA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_jpeg::zune_core::bytestream::ZCursor::new(bytes),
        options,
    );
    decoder.decode_headers().map_err(decode_error)?;
    let info = decoder
        .info()
        .ok_or_else(|| message_error("JPEG headers were not read"))?;
    let (source_w, source_h) = (u32::from(info.width), u32::from(info.height));
    let bgra = decoder.decode().map_err(decode_error)?;
    let (target_w, target_h) = fit_dims(source_w, source_h, edge);
    let bgra = resize_bgra(bgra, source_w, source_h, target_w, target_h)?;
    render_bgra(bgra, target_w, target_h)
}

/// Decodes a non-JPEG raster through `image`, resizes with SIMD and swaps the
/// channels to BGRA in one pass.
fn decode_raster(
    bytes: &[u8],
    format: image::ImageFormat,
    edge: u32,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let decoded = image::load_from_memory_with_format(bytes, format).map_err(image_error)?;
    let (source_w, source_h) = (decoded.width(), decoded.height());
    let rgba = decoded.into_rgba8();
    let (target_w, target_h) = fit_dims(source_w, source_h, edge);
    let resized = resize_u8x4(rgba.into_raw(), source_w, source_h, target_w, target_h)?;
    let mut bgra = resized;
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    render_bgra(bgra, target_w, target_h)
}

/// Longest edge `edge`, preserving aspect ratio, never upscaling.
fn fit_dims(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= edge || long == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(edge) / f64::from(long);
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

/// Wraps a BGRA buffer in the [`RenderImage`] GPUI expects.
fn render_bgra(
    bgra: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, bgra)
        .ok_or_else(|| message_error("decoded cover buffer has the wrong length"))?;
    Ok(Arc::new(RenderImage::new(smallvec::smallvec![Frame::new(
        buffer
    )])))
}

/// Resizes an RGB8 buffer with the SIMD resizer.
fn resize_rgb(
    rgb: Vec<u8>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<Vec<u8>, ImageCacheError> {
    if (source_w, source_h) == (target_w, target_h) {
        return Ok(rgb);
    }
    let source = fast_image_resize::images::Image::from_vec_u8(
        source_w,
        source_h,
        rgb,
        fast_image_resize::PixelType::U8x3,
    )
    .map_err(decode_error)?;
    let mut target = fast_image_resize::images::Image::new(
        target_w,
        target_h,
        fast_image_resize::PixelType::U8x3,
    );
    fast_image_resize::Resizer::new()
        .resize(
            &source,
            &mut target,
            &fast_image_resize::ResizeOptions::new().resize_alg(
                fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Box),
            ),
        )
        .map_err(decode_error)?;
    Ok(target.into_vec())
}

/// Resizes an RGB8 buffer with the SIMD resizer.
fn resize_u8x4(
    rgba: Vec<u8>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<Vec<u8>, ImageCacheError> {
    if (source_w, source_h) == (target_w, target_h) {
        return Ok(rgba);
    }
    let source = fast_image_resize::images::Image::from_vec_u8(
        source_w,
        source_h,
        rgba,
        fast_image_resize::PixelType::U8x4,
    )
    .map_err(decode_error)?;
    let mut target = fast_image_resize::images::Image::new(
        target_w,
        target_h,
        fast_image_resize::PixelType::U8x4,
    );
    fast_image_resize::Resizer::new()
        .resize(
            &source,
            &mut target,
            &fast_image_resize::ResizeOptions::new().resize_alg(
                fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Box),
            ),
        )
        .map_err(decode_error)?;
    Ok(target.into_vec())
}

/// Resizes a BGRA buffer with the SIMD resizer.
fn resize_bgra(
    bgra: Vec<u8>,
    source_w: u32,
    source_h: u32,
    target_w: u32,
    target_h: u32,
) -> Result<Vec<u8>, ImageCacheError> {
    resize_u8x4(bgra, source_w, source_h, target_w, target_h)
}

/// Swaps one RGB buffer into the BGRA order GPUI paints with, adding opaque
/// alpha. Four-byte writes keep the pass linear and branch-free.
fn rgb_to_bgra(rgb: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgb.len() / 3 * 4);
    for pixel in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], u8::MAX]);
    }
    bgra
}

/// Decodes a display derivative. Derivatives are always JPEG and already at
/// the requested bucket edge, so this is a direct BGRA decode with no resize.
fn decode_derivative(path: &Path) -> Result<Arc<RenderImage>, ImageCacheError> {
    let bytes = std::fs::read(path).map_err(io_error)?;
    let options = zune_jpeg::zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::BGRA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_jpeg::zune_core::bytestream::ZCursor::new(&bytes),
        options,
    );
    decoder.decode_headers().map_err(decode_error)?;
    let info = decoder
        .info()
        .ok_or_else(|| message_error("derivative JPEG headers were not read"))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    let bgra = decoder.decode().map_err(decode_error)?;
    render_bgra(bgra, width, height)
}

/// Writes the decoded display image to the derivative cache.
fn write_derivative(
    source: &Path,
    edge: u32,
    dir: &Path,
    image: &Arc<RenderImage>,
) -> Result<(), ImageCacheError> {
    let Some((width, height)) = derivative_dims(image) else {
        return Ok(());
    };
    let Some(bytes) = image.as_bytes(0) else {
        return Ok(());
    };
    let mut rgb = Vec::with_capacity(bytes.len() / 4 * 3);
    for pixel in bytes.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, DERIVATIVE_QUALITY)
        .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(image_error)?;
    let name = derivative_name(source, edge);
    std::fs::create_dir_all(dir).map_err(io_error)?;
    crate::cache::write_atomic(dir, &name, &encoded).map_err(anyhow_error)?;
    Ok(())
}

/// Dimensions of the first frame of a decoded image.
fn derivative_dims(image: &Arc<RenderImage>) -> Option<(u32, u32)> {
    if image.frame_count() == 0 {
        return None;
    }
    let size = image.size(0);
    Some((u32::from(size.width).max(1), u32::from(size.height).max(1)))
}

/// File name of the cached derivative for `source` at `edge`.
fn derivative_name(source: &Path, edge: u32) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    if let Ok(metadata) = std::fs::metadata(source) {
        metadata.len().hash(&mut hasher);
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            duration.as_nanos().hash(&mut hasher);
        }
    }
    DERIVATIVE_VERSION.hash(&mut hasher);
    edge.hash(&mut hasher);
    format!("cover-{:016x}-{edge}.jpg", hasher.finish())
}

/// Path of the cached derivative for `source` at `edge`, if one exists.
fn derivative_path(source: &Path, edge: u32, dir: &Path) -> Option<PathBuf> {
    let path = dir.join(derivative_name(source, edge));
    path.is_file().then_some(path)
}

/// Directory holding display derivatives, or `None` when no cache root is
/// configured (the app then decodes originals as before).
fn derivative_root() -> Option<PathBuf> {
    if std::env::var("MUSIKA_COVER_DERIVATIVES").is_ok_and(|value| value == "0") {
        return None;
    }
    if let Ok(dir) = std::env::var("MUSIKA_UI_COVER_CACHE_DIR")
        && !dir.trim().is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(cache.join("musika").join("cover-derivatives"))
}

fn io_error(err: std::io::Error) -> ImageCacheError {
    ImageCacheError::Io(Arc::new(err))
}

fn image_error(err: image::ImageError) -> ImageCacheError {
    ImageCacheError::Image(Arc::new(err))
}

/// An error caused by a decoder/resizer type that implements `Error`.
fn decode_error(err: impl std::error::Error + Send + Sync + 'static) -> ImageCacheError {
    ImageCacheError::Other(Arc::new(anyhow::Error::new(err)))
}

/// An error described by a message (invalid state, unsupported format).
fn message_error(message: impl Into<String>) -> ImageCacheError {
    ImageCacheError::Other(Arc::new(anyhow::anyhow!(message.into())))
}

/// An error already carrying its context.
fn anyhow_error(err: anyhow::Error) -> ImageCacheError {
    ImageCacheError::Other(Arc::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "musika-fast-image-{}-{stamp}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_test_jpeg(path: &Path, width: u32, height: u32) {
        let image = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8])
        });
        image
            .save_with_format(path, image::ImageFormat::Jpeg)
            .expect("save test jpeg");
    }

    fn display_edge(image: &Arc<RenderImage>) -> (u32, u32) {
        let size = image.size(0);
        (u32::from(size.width), u32::from(size.height))
    }

    #[test]
    fn edge_buckets_round_up_and_cap() {
        assert_eq!(quantize_edge(1), 64);
        assert_eq!(quantize_edge(64), 64);
        assert_eq!(quantize_edge(65), 96);
        assert_eq!(quantize_edge(426), 512);
        assert_eq!(quantize_edge(680), 768);
        assert_eq!(quantize_edge(4096), 4096);
        assert_eq!(quantize_edge(9000), 4096);
    }

    #[test]
    fn large_jpegs_decode_to_the_bucket_edge_without_upscaling() {
        let dir = temp_dir("large-jpeg");
        let path = dir.join("large.jpg");
        write_test_jpeg(&path, 1600, 1200);

        let image = decode_for_display_in(&path, 512, Some(&dir)).expect("decode");
        let (width, height) = display_edge(&image);
        assert!(width.max(height) <= 512);
        assert_eq!((width, height), (512, 384));
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn small_jpegs_never_upscale() {
        let dir = temp_dir("small-jpeg");
        let path = dir.join("small.jpg");
        write_test_jpeg(&path, 320, 240);

        let image = decode_for_display_in(&path, 512, None).expect("decode");
        assert_eq!(display_edge(&image), (320, 240));
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bgra_channel_order_matches_gpui() {
        let dir = temp_dir("bgra");
        let path = dir.join("red.jpg");
        image::RgbImage::from_fn(64, 64, |_, _| image::Rgb([255, 0, 0]))
            .save_with_format(&path, image::ImageFormat::Jpeg)
            .expect("save");
        let image = decode_for_display_in(&path, 64, None).expect("decode");
        let bytes = image.as_bytes(0).expect("frame bytes");
        let pixel = &bytes[..4];
        assert!(pixel[0] < 24, "blue channel: {pixel:?}");
        assert!(pixel[1] < 24, "green channel: {pixel:?}");
        assert!(pixel[2] > 220, "red channel: {pixel:?}");
        assert_eq!(pixel[3], u8::MAX);
        _ = std::fs::remove_dir_all(dir);
    }

    /// DCT scaling trades some high-frequency detail for speed; the result
    /// must still match a full decode followed by the same downscale closely
    /// enough that covers do not visibly change.
    #[test]
    fn scaled_decode_stays_close_to_the_full_decode() {
        let dir = temp_dir("quality");
        let path = dir.join("cover.jpg");
        write_test_jpeg(&path, 1600, 1200);

        let fast = decode_for_display_in(&path, 512, None).expect("fast decode");
        let (fast_w, fast_h) = display_edge(&fast);
        let reference = image::open(&path).expect("reference decode");
        let mut reference = reference
            .thumbnail(fast_w.max(fast_h), fast_w.max(fast_h))
            .into_rgba8();
        for pixel in reference.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        let reference = RenderImage::new(smallvec::smallvec![Frame::new(reference)]);
        let (reference_w, reference_h) = (
            reference.size(0).width.0 as u32,
            reference.size(0).height.0 as u32,
        );

        assert_eq!((fast_w, fast_h), (reference_w, reference_h));
        let fast_bytes = fast.as_bytes(0).expect("fast frame");
        let reference_bytes = reference.as_bytes(0).expect("reference frame");
        let channels = (fast_w * fast_h) as u64 * 3;
        let mut total = 0u64;
        for (fast_pixel, reference_pixel) in fast_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .zip(reference_bytes.as_chunks::<4>().0)
        {
            for channel in 0..3 {
                total += u64::from(fast_pixel[channel].abs_diff(reference_pixel[channel]));
            }
        }
        let mean_abs_diff = total as f64 / channels as f64;
        assert!(
            mean_abs_diff < 12.0,
            "scaled decode differs from the full decode by {mean_abs_diff:.2}/255"
        );
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn second_open_uses_the_display_derivative() {
        let dir = temp_dir("derivative");
        let path = dir.join("cover.jpg");
        write_test_jpeg(&path, 1600, 1600);

        let first = decode_for_display_in(&path, 512, Some(&dir)).expect("first decode");
        let derivative =
            derivative_path(&path, quantize_edge(512), &dir).expect("derivative written");
        assert!(derivative.is_file());
        let second = decode_for_display_in(&path, 512, Some(&dir)).expect("second decode");
        assert_eq!(display_edge(&first), display_edge(&second));
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn derivative_is_keyed_on_the_source_mtime() {
        let dir = temp_dir("derivative-key");
        let path = dir.join("cover.jpg");
        write_test_jpeg(&path, 1600, 1600);
        let _ = decode_for_display_in(&path, 512, Some(&dir)).expect("decode");
        let original = derivative_path(&path, quantize_edge(512), &dir).expect("derivative");

        std::thread::sleep(std::time::Duration::from_millis(10));
        write_test_jpeg(&path, 1600, 1600);
        // The mtime changed, so the old derivative must not be reused.
        assert_ne!(
            derivative_path(&path, quantize_edge(512), &dir),
            Some(original)
        );
        _ = std::fs::remove_dir_all(dir);
    }
}
