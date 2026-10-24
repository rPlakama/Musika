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

//! Seed extraction and the DB6 seed cache (THM-6 R1).
//!
//! The seed — the quantized, scored source color — is the expensive step
//! (decode, downscale, quantize, score). It is cached in `color_seed` keyed by
//! `cover_hash | source_mode | engine` with a small LRU, so variant, spec or
//! contrast changes re-generate the palette without touching the image again.
//!
//! Two extraction modes, following DMS (`sourcecolor.go` / caelestia):
//! `dominant` uses the crate's `Score` (matugen's own pick); `colorful` ports
//! caelestia's scoring, which prefers a vivid, well-lit color over the most
//! common muted one.

use std::path::Path;

use crate::consts::now_secs;
use anyhow::{Context, Result};
use material_colors::color::Rgb;
use material_colors::dislike::fix_if_disliked;
use material_colors::hct::Hct;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

/// Thumbnail side fed to the quantizer (matugen uses 112).
const THUMBNAIL: u32 = 112;
/// Refuse pathological art before decoding it: the palette reads 112 px, so an
/// oversized upload must not become a multi-hundred-megabyte allocation on a
/// blocking worker (the caller keeps the static palette instead).
const MAX_SOURCE_EDGE: u32 = 8192;
/// Maximum quantizer clusters (matugen uses 128).
const MAX_COLORS: usize = 128;
/// How many seeds the DB6 cache keeps before evicting the least recently used.
const SEED_CACHE_LIMIT: i64 = 32;

/// Extraction pipeline version; a change here invalidates cached seeds.
pub const SEED_ENGINE: &str = "seed-v1";

// caelestia scoring weights, ported from DMS `sourcecolor.go`.
const SCORE_TARGET_CHROMA: f64 = 48.0;
const SCORE_WEIGHT_PROPORTION: f64 = 0.7;
const SCORE_WEIGHT_CHROMA_ABOVE: f64 = 0.3;
const SCORE_WEIGHT_CHROMA_BELOW: f64 = 0.1;

name_enum! {
    /// Where the seed comes from.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum SourceMode {
        /// matugen's own `Score` pick (the historical default).
        #[default]
        Dominant => "dominant" | "default",
        /// caelestia-style vivid scoring.
        Colorful => "colorful",
    }
}

/// A resolved seed plus the image colorfulness (drives the `smart` variant).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seed {
    pub color: Rgb,
    pub colorfulness: f64,
    /// `dominant`, `colorful` or `seed` (explicit override).
    pub mode: &'static str,
}

/// Decodes `path`, downscales to a 112² thumbnail and keeps opaque pixels.
///
/// The caller normally hands us the cover cache's 320 px thumbnail (see
/// [`crate::covers::palette_path`]). When it has to fall back to the original,
/// the dimension probe still rejects absurd uploads before the decoder
/// allocates their pixels, and [`image::DynamicImage::thumbnail`] samples the
/// small output instead of running a full-kernel resize over the source.
fn sample(path: &Path) -> Result<Vec<Rgb>> {
    let context = || format!("failed to decode cover {}", path.display());
    let (width, height) = image::ImageReader::open(path)
        .with_context(context)?
        .with_guessed_format()
        .with_context(context)?
        .into_dimensions()
        .with_context(context)?;
    anyhow::ensure!(
        width <= MAX_SOURCE_EDGE && height <= MAX_SOURCE_EDGE,
        "cover {} is {width}x{height}, over the {MAX_SOURCE_EDGE} px palette limit",
        path.display()
    );

    let image = image::ImageReader::open(path)
        .with_context(context)?
        .with_guessed_format()
        .with_context(context)?
        .decode()
        .with_context(context)?;
    let thumbnail = image.thumbnail(THUMBNAIL, THUMBNAIL).to_rgba8();

    Ok(thumbnail
        .pixels()
        .filter(|pixel| pixel.0[3] == 255)
        .map(|pixel| Rgb::new(pixel.0[0], pixel.0[1], pixel.0[2]))
        .collect())
}

/// matugen's own pick: quantize, drop low-chroma/rare clusters, take the
/// highest scored color.
fn dominant(pixels: &[Rgb]) -> Result<Rgb> {
    let population = QuantizerCelebi::quantize(pixels, MAX_COLORS);
    let ranked = Score::score(&population.color_to_count, None, None, Some(true));
    ranked
        .first()
        .copied()
        .context("cover produced no suitable seed color")
}

/// caelestia's pick (DMS `scoreColors`): no chroma filtering, hue excitement
/// windows and a descending chroma/tone cutoff, so a small vivid region beats a
/// large muted one.
fn colorful(pixels: &[Rgb]) -> Result<Rgb> {
    let population = QuantizerCelebi::quantize(pixels, MAX_COLORS);

    // Sorted so the stable sort below and the cutoff scan resolve ties the
    // same way every run; the quantizer's map order is not a contract.
    let mut keys: Vec<Rgb> = population.color_to_count.keys().copied().collect();
    keys.sort_by_key(|color| super::rgb::argb(*color));

    let mut hue_population = [0u32; 360];
    let mut total = 0u64;
    let mut colors: Vec<Hct> = Vec::with_capacity(keys.len());
    for key in keys {
        let count = population.color_to_count.get(&key).copied().unwrap_or(0);
        let hct: Hct = key.into();
        hue_population[normalize_degree(hct.get_hue() as i32)] += count;
        total += u64::from(count);
        colors.push(hct);
    }
    if total == 0 {
        anyhow::bail!("cover produced no usable seed color");
    }

    // Hues with more usage in a neighboring 30 degree slice score higher.
    let mut excited = [0.0f64; 360];
    for (hue, count) in hue_population.iter().enumerate() {
        let proportion = f64::from(*count) / total as f64;
        for i in hue as i32 - 14..hue as i32 + 16 {
            excited[normalize_degree(i)] += proportion;
        }
    }

    let mut scored: Vec<(Hct, f64)> = colors
        .iter()
        .map(|hct| {
            let proportion = excited[normalize_degree(hct.get_hue().round() as i32)];
            let chroma_weight = if hct.get_chroma() < SCORE_TARGET_CHROMA {
                SCORE_WEIGHT_CHROMA_BELOW
            } else {
                SCORE_WEIGHT_CHROMA_ABOVE
            };
            let score = proportion * 100.0 * SCORE_WEIGHT_PROPORTION
                + (hct.get_chroma() - SCORE_TARGET_CHROMA) * chroma_weight;
            (*hct, score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    for cutoff in (0..=20).rev() {
        let cutoff = f64::from(cutoff);
        for (hct, _) in &scored {
            if hct.get_chroma() > cutoff && hct.get_tone() > cutoff * 3.0 {
                return Ok(fix_if_disliked(*hct).into());
            }
        }
    }
    anyhow::bail!("cover produced no usable seed color")
}

/// Extracts the seed for `mode` synchronously (decode + score).
pub fn extract(path: &Path, mode: SourceMode) -> Result<(Rgb, f64)> {
    let pixels = sample(path)?;
    if pixels.is_empty() {
        anyhow::bail!("cover has no opaque pixels");
    }
    let color = match mode {
        SourceMode::Dominant => dominant(&pixels)?,
        SourceMode::Colorful => colorful(&pixels)?,
    };
    Ok((color, colorfulness(&pixels)))
}

/// Resolves the seed for a cover: explicit override, DB6 cache, or extraction.
pub async fn resolve(
    pool: &SqlitePool,
    cover_hash: &str,
    mode: SourceMode,
    path: &Path,
    override_seed: Option<u32>,
) -> Result<Seed> {
    if let Some(argb) = override_seed {
        return Ok(Seed {
            color: super::rgb::from_argb(argb),
            colorfulness: 0.0,
            mode: "seed",
        });
    }

    if let Some((argb, colorfulness)) = load(pool, cover_hash, mode).await? {
        touch(pool, cover_hash, mode).await?;
        return Ok(Seed {
            color: super::rgb::from_argb(argb),
            colorfulness,
            mode: mode.name(),
        });
    }

    let path = path.to_path_buf();
    let (color, colorfulness) = tokio::task::spawn_blocking(move || extract(&path, mode))
        .await
        .context("seed extraction task panicked")??;

    store(pool, cover_hash, mode, color, colorfulness).await?;
    prune(pool).await?;
    Ok(Seed {
        color,
        colorfulness,
        mode: mode.name(),
    })
}

// ---------------------------------------------------------------------------
// DB6 seed cache
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct SeedRow {
    seed: i64,
    colorfulness: f64,
}

async fn load(pool: &SqlitePool, cover_hash: &str, mode: SourceMode) -> Result<Option<(u32, f64)>> {
    let row = sqlx::query_as::<_, SeedRow>(
        "SELECT seed, colorfulness FROM color_seed
         WHERE cover_hash = ?1 AND mode = ?2 AND engine = ?3",
    )
    .bind(cover_hash)
    .bind(mode.name())
    .bind(SEED_ENGINE)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| (row.seed as u32, row.colorfulness)))
}

async fn touch(pool: &SqlitePool, cover_hash: &str, mode: SourceMode) -> Result<()> {
    sqlx::query(
        "UPDATE color_seed SET used = ?4
         WHERE cover_hash = ?1 AND mode = ?2 AND engine = ?3",
    )
    .bind(cover_hash)
    .bind(mode.name())
    .bind(SEED_ENGINE)
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(())
}

async fn store(
    pool: &SqlitePool,
    cover_hash: &str,
    mode: SourceMode,
    color: Rgb,
    colorfulness: f64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO color_seed (cover_hash, mode, engine, seed, colorfulness, used)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(cover_hash, mode, engine) DO UPDATE SET
             seed = excluded.seed,
             colorfulness = excluded.colorfulness,
             used = excluded.used",
    )
    .bind(cover_hash)
    .bind(mode.name())
    .bind(SEED_ENGINE)
    .bind(super::rgb::argb(color) as i64)
    .bind(colorfulness)
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(())
}

/// Drops the least recently used seeds beyond the cache bound.
async fn prune(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "DELETE FROM color_seed WHERE rowid NOT IN
             (SELECT rowid FROM color_seed ORDER BY used DESC LIMIT ?1)",
    )
    .bind(SEED_CACHE_LIMIT)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Hasler-Süsstrunk colorfulness of the sampled pixels (0-255 scale):
/// `sqrt(σrg² + σyb²) + 0.3·sqrt(μrg² + μyb²)`.
fn colorfulness(pixels: &[Rgb]) -> f64 {
    if pixels.is_empty() {
        return 0.0;
    }
    let count = pixels.len() as f64;

    let mut mean_rg = 0.0;
    let mut mean_yb = 0.0;
    for pixel in pixels {
        let (r, g, b) = (
            f64::from(pixel.red),
            f64::from(pixel.green),
            f64::from(pixel.blue),
        );
        mean_rg += r - g;
        mean_yb += 0.5 * (r + g) - b;
    }
    mean_rg /= count;
    mean_yb /= count;

    let mut variance_rg = 0.0;
    let mut variance_yb = 0.0;
    for pixel in pixels {
        let (r, g, b) = (
            f64::from(pixel.red),
            f64::from(pixel.green),
            f64::from(pixel.blue),
        );
        let rg = r - g - mean_rg;
        let yb = 0.5 * (r + g) - b - mean_yb;
        variance_rg += rg * rg;
        variance_yb += yb * yb;
    }
    let sigma_rg = (variance_rg / count).sqrt();
    let sigma_yb = (variance_yb / count).sqrt();

    (sigma_rg.powi(2) + sigma_yb.powi(2)).sqrt() + 0.3 * (mean_rg.powi(2) + mean_yb.powi(2)).sqrt()
}

fn normalize_degree(degree: i32) -> usize {
    degree.rem_euclid(360) as usize
}
