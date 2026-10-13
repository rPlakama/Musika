//! Dynamic color engine (THM-1 / roadmap T0): cover art → Material You dark
//! scheme.
//!
//! matugen itself is GPL-2.0; only its engine lineage matters here. We embed
//! the MIT/Apache `material-colors` crate and reimplement its pipeline:
//!
//!   decode → downscale to 112² (Triangle) → keep opaque pixels →
//!   `QuantizerCelebi` (128 clusters) → `Score` (drops CAM16 chroma < 5 and
//!   rare hues) → first seed → `SchemeTonalSpot` dark scheme → role map.
//!
//! No DB and no QML at this stage: `colors::fetch` (DB6 cache) and the palette
//! singleton are wired in later milestones.

#![allow(dead_code)] // T0 spike: consumed by the DB/cache wiring at T1.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use image::imageops::FilterType;
use material_colors::color::Rgb;
use material_colors::dynamic_color::DynamicScheme;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;

use crate::db::Dbs;

/// Thumbnail side fed to the quantizer (matugen uses 112).
const THUMBNAIL: u32 = 112;
/// Maximum quantizer clusters (matugen uses 128).
const MAX_COLORS: usize = 128;

/// Which scheme family to build. `Smart` picks per image (T4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VariantChoice {
    /// Choose by Hasler-Süsstrunk colorfulness (the default).
    #[default]
    Smart,
    TonalSpot,
    Vibrant,
    Expressive,
    Neutral,
    Monochrome,
    Fidelity,
    Content,
    Rainbow,
    FruitSalad,
}

impl VariantChoice {
    /// Reads `MUSIKA_COLOR_VARIANT` (`smart` by default).
    pub fn from_env() -> Self {
        match std::env::var("MUSIKA_COLOR_VARIANT")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "tonal_spot" => Self::TonalSpot,
            "vibrant" => Self::Vibrant,
            "expressive" => Self::Expressive,
            "neutral" => Self::Neutral,
            "monochrome" => Self::Monochrome,
            "fidelity" => Self::Fidelity,
            "content" => Self::Content,
            "rainbow" => Self::Rainbow,
            "fruit_salad" => Self::FruitSalad,
            _ => Self::Smart,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Smart => "smart",
            Self::TonalSpot => "tonal_spot",
            Self::Vibrant => "vibrant",
            Self::Expressive => "expressive",
            Self::Neutral => "neutral",
            Self::Monochrome => "monochrome",
            Self::Fidelity => "fidelity",
            Self::Content => "content",
            Self::Rainbow => "rainbow",
            Self::FruitSalad => "fruit_salad",
        }
    }

    /// Resolves the concrete scheme for an image's colorfulness.
    fn resolve(self, colorfulness: f64) -> material_colors::dynamic_color::Variant {
        use material_colors::dynamic_color::Variant;
        match self {
            Self::Smart => smart_variant(colorfulness),
            Self::TonalSpot => Variant::TonalSpot,
            Self::Vibrant => Variant::Vibrant,
            Self::Expressive => Variant::Expressive,
            Self::Neutral => Variant::Neutral,
            Self::Monochrome => Variant::Monochrome,
            Self::Fidelity => Variant::Fidelity,
            Self::Content => Variant::Content,
            Self::Rainbow => Variant::Rainbow,
            Self::FruitSalad => Variant::FruitSalad,
        }
    }
}

/// matugen's `smart_scheme` mapping (dark-only): loud covers get a vibrant
/// scheme, flat ones settle for neutral or monochrome.
fn smart_variant(colorfulness: f64) -> material_colors::dynamic_color::Variant {
    use material_colors::dynamic_color::Variant;
    if colorfulness > 70.0 {
        Variant::Vibrant
    } else if colorfulness >= 20.0 {
        Variant::TonalSpot
    } else if colorfulness >= 6.0 {
        Variant::Neutral
    } else {
        Variant::Monochrome
    }
}

/// Generation knobs (`MUSIKA_COLOR_VARIANT` / `MUSIKA_COLOR_CONTRAST`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub variant: VariantChoice,
    pub contrast: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            variant: VariantChoice::Smart,
            contrast: 0.0,
        }
    }
}

impl Options {
    pub fn from_env() -> Self {
        let contrast = std::env::var("MUSIKA_COLOR_CONTRAST")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .filter(|value: &f64| (-1.0..=1.0).contains(value))
            .unwrap_or(0.0);
        Self {
            variant: VariantChoice::from_env(),
            contrast,
        }
    }

    /// Cache tag: a knob change invalidates stored schemes.
    pub fn generator(&self) -> String {
        format!(
            "material-colors-0.5/{}/{:+.2}",
            self.variant.name(),
            self.contrast
        )
    }
}

/// A generated dark scheme for one cover.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverScheme {
    /// Variant name (`tonal_spot` for now).
    pub scheme: String,
    /// ARGB of the scored source color.
    pub seed: u32,
    /// Role name -> `#rrggbb`.
    pub roles: BTreeMap<String, String>,
}

impl CoverScheme {
    /// Payload used by the cache and the `colors` dataReady channel.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "scheme": self.scheme,
            "seed": self.seed,
            "roles": self.roles,
        })
        .to_string()
    }
}

/// Generates the scheme from a cover file (jpg/png/webp/gif) with the given
/// knobs. The variant is resolved per image when `Options::variant` is
/// `Smart` (Hasler-Süsstrunk colorfulness).
pub fn generate_from_path(path: &Path, options: &Options) -> Result<CoverScheme> {
    let image = image::open(path)
        .with_context(|| format!("failed to decode cover {}", path.display()))?;
    let thumbnail = image
        .resize_exact(THUMBNAIL, THUMBNAIL, FilterType::Triangle)
        .to_rgba8();

    let pixels: Vec<Rgb> = thumbnail
        .pixels()
        .filter(|pixel| pixel.0[3] == 255)
        .map(|pixel| Rgb::new(pixel.0[0], pixel.0[1], pixel.0[2]))
        .collect();

    if pixels.is_empty() {
        anyhow::bail!("cover has no opaque pixels");
    }

    let quantized = QuantizerCelebi::quantize(&pixels, MAX_COLORS);
    let ranked = Score::score(&quantized.color_to_count, None, None, Some(true));
    let seed = ranked
        .first()
        .copied()
        .context("cover produced no suitable seed color")?;

    let variant = options.variant.resolve(colorfulness(&pixels));
    Ok(scheme_from_seed(seed, variant, options.contrast))
}

/// Builds a dark scheme for an already scored seed and concrete variant.
pub fn scheme_from_seed(
    seed: Rgb,
    variant: material_colors::dynamic_color::Variant,
    contrast: f64,
) -> CoverScheme {
    let dynamic = DynamicScheme::by_variant(seed, &variant, true, Some(contrast));

    CoverScheme {
        scheme: variant_name(&variant).to_string(),
        seed: seed.as_argb_u32(),
        roles: role_map(&dynamic),
    }
}

/// Convenience wrapper: TonalSpot at standard contrast (used by tests and any
/// caller that wants the plain MY3 scheme).
pub fn from_seed(seed: Rgb) -> CoverScheme {
    scheme_from_seed(seed, material_colors::dynamic_color::Variant::TonalSpot, 0.0)
}

fn variant_name(variant: &material_colors::dynamic_color::Variant) -> &'static str {
    use material_colors::dynamic_color::Variant;
    match variant {
        Variant::Monochrome => "monochrome",
        Variant::Neutral => "neutral",
        Variant::TonalSpot => "tonal_spot",
        Variant::Vibrant => "vibrant",
        Variant::Expressive => "expressive",
        Variant::Fidelity => "fidelity",
        Variant::Content => "content",
        Variant::Rainbow => "rainbow",
        Variant::FruitSalad => "fruit_salad",
        Variant::Cmf => "cmf",
    }
}

/// Hasler-Süsstrunk colorfulness of the quantizer pixels (0-255 scale):
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

    (sigma_rg.powi(2) + sigma_yb.powi(2)).sqrt()
        + 0.3 * (mean_rg.powi(2) + mean_yb.powi(2)).sqrt()
}

// ---------------------------------------------------------------------------
// DB6 cache (THM-2)
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct CachedRow {
    cover_hash: String,
    scheme: String,
    seed: i64,
    dark: String,
    generator: String,
}

/// Returns the scheme for a cached cover, generating and storing it when the
/// row is missing, stale (cover hash changed) or from older knobs.
/// `None` when the cover itself is not cached (no art → static palette).
pub async fn fetch(dbs: &Dbs, cover_art: &str) -> Result<Option<CoverScheme>> {
    let Some(path) = crate::covers::cached_path(&dbs.covers, cover_art).await? else {
        return Ok(None);
    };
    let Some(hash) = crate::covers::hash(&dbs.covers, cover_art).await? else {
        return Ok(None);
    };
    let options = Options::from_env();
    Ok(Some(
        fetch_into(&dbs.colors, cover_art, &path, &hash, &options).await?,
    ))
}

/// Cache-aware generation against explicit cover bytes; also the test seam.
pub async fn fetch_into(
    pool: &SqlitePool,
    cover_art: &str,
    path: &Path,
    cover_hash: &str,
    options: &Options,
) -> Result<CoverScheme> {
    let generator = options.generator();
    if let Some(row) = load(pool, cover_art).await? {
        if row.cover_hash == cover_hash && row.generator == generator {
            let roles = serde_json::from_str(&row.dark)
                .with_context(|| format!("stored colors for {cover_art} are malformed"))?;
            return Ok(CoverScheme {
                scheme: row.scheme,
                seed: row.seed as u32,
                roles,
            });
        }
    }

    let source = path.to_path_buf();
    let knobs = *options;
    let scheme = tokio::task::spawn_blocking(move || generate_from_path(&source, &knobs))
        .await
        .context("color generation task panicked")??;
    store(pool, cover_art, cover_hash, &generator, &scheme).await?;
    Ok(scheme)
}

/// Drops the cached scheme for one cover, forcing the next request to
/// regenerate (manual `colors_refresh`).
pub async fn invalidate(dbs: &Dbs, cover_art: &str) -> Result<()> {
    invalidate_with(&dbs.colors, cover_art).await
}

pub async fn invalidate_with(pool: &SqlitePool, cover_art: &str) -> Result<()> {
    sqlx::query("DELETE FROM cover_color WHERE cover_art = ?1")
        .bind(cover_art)
        .execute(pool)
        .await?;
    Ok(())
}

async fn load(pool: &SqlitePool, cover_art: &str) -> Result<Option<CachedRow>> {
    let row = sqlx::query_as::<_, CachedRow>(
        "SELECT cover_hash, scheme, seed, dark, generator
         FROM cover_color WHERE cover_art = ?1",
    )
    .bind(cover_art)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

async fn store(
    pool: &SqlitePool,
    cover_art: &str,
    cover_hash: &str,
    generator: &str,
    scheme: &CoverScheme,
) -> Result<()> {
    let dark = serde_json::to_string(&scheme.roles)?;
    sqlx::query(
        "INSERT INTO cover_color (cover_art, cover_hash, scheme, seed, dark, generator, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(cover_art) DO UPDATE SET
             cover_hash = excluded.cover_hash,
             scheme = excluded.scheme,
             seed = excluded.seed,
             dark = excluded.dark,
             generator = excluded.generator,
             generated_at = excluded.generated_at",
    )
    .bind(cover_art)
    .bind(cover_hash)
    .bind(&scheme.scheme)
    .bind(scheme.seed as i64)
    .bind(dark)
    .bind(generator)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes cached schemes whose cover id vanished from covers.db.
pub async fn purge(dbs: &Dbs) -> Result<usize> {
    purge_with(&dbs.colors, &dbs.covers).await
}

pub async fn purge_with(colors: &SqlitePool, covers: &SqlitePool) -> Result<usize> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT cover_art FROM cover_color")
        .fetch_all(colors)
        .await?;
    if ids.is_empty() {
        return Ok(0);
    }

    let existing: std::collections::HashSet<String> = sqlx::query_scalar("SELECT id FROM covers")
        .fetch_all(covers)
        .await?
        .into_iter()
        .collect();

    let stale: Vec<&String> = ids.iter().filter(|id| !existing.contains(*id)).collect();
    if stale.is_empty() {
        return Ok(0);
    }

    let mut tx = colors.begin().await?;
    for id in &stale {
        sqlx::query("DELETE FROM cover_color WHERE cover_art = ?1")
            .bind(*id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(stale.len())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// The curated role subset: everything `Theme.js` needs, and nothing
/// fixed/dim that the dark-only app will not use.
fn role_map(dynamic: &DynamicScheme) -> BTreeMap<String, String> {
    let mut roles = BTreeMap::new();
    let mut put = |name: &str, color: Rgb| {
        roles.insert(name.to_string(), hex(color));
    };

    put("primary", dynamic.primary());
    put("on_primary", dynamic.on_primary());
    put("primary_container", dynamic.primary_container());
    put("on_primary_container", dynamic.on_primary_container());
    put("secondary", dynamic.secondary());
    put("on_secondary", dynamic.on_secondary());
    put("secondary_container", dynamic.secondary_container());
    put("on_secondary_container", dynamic.on_secondary_container());
    put("tertiary", dynamic.tertiary());
    put("on_tertiary", dynamic.on_tertiary());
    put("tertiary_container", dynamic.tertiary_container());
    put("on_tertiary_container", dynamic.on_tertiary_container());
    put("error", dynamic.error());
    put("on_error", dynamic.on_error());
    put("error_container", dynamic.error_container());
    put("on_error_container", dynamic.on_error_container());
    put("background", dynamic.background());
    put("on_background", dynamic.on_background());
    put("surface", dynamic.surface());
    put("surface_dim", dynamic.surface_dim());
    put("surface_bright", dynamic.surface_bright());
    put("surface_container_lowest", dynamic.surface_container_lowest());
    put("surface_container_low", dynamic.surface_container_low());
    put("surface_container", dynamic.surface_container());
    put("surface_container_high", dynamic.surface_container_high());
    put("surface_container_highest", dynamic.surface_container_highest());
    put("on_surface", dynamic.on_surface());
    put("surface_variant", dynamic.surface_variant());
    put("on_surface_variant", dynamic.on_surface_variant());
    put("inverse_surface", dynamic.inverse_surface());
    put("inverse_on_surface", dynamic.inverse_on_surface());
    put("outline", dynamic.outline());
    put("outline_variant", dynamic.outline_variant());
    put("scrim", dynamic.scrim());
    put("surface_tint", dynamic.surface_tint());

    roles
}

fn hex(color: Rgb) -> String {
    format!("#{}", color.as_hex())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use image::{Rgba, RgbaImage};

    /// Writes a synthetic 32x32 cover and returns its path.
    fn write_cover(name: &str, paint: impl Fn(u32, u32) -> Rgba<u8>) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "musika-cover-{}-{name}.png",
            std::process::id()
        ));
        let mut image = RgbaImage::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                image.put_pixel(x, y, paint(x, y));
            }
        }
        image.save(&path).unwrap();
        path
    }

    /// Fixed knobs so variant assertions do not depend on smart selection.
    fn tonal() -> Options {
        Options {
            variant: VariantChoice::TonalSpot,
            contrast: 0.0,
        }
    }

    #[test]
    fn generated_scheme_exposes_the_full_role_map() {
        let path = write_cover("blue", |_, _| Rgba([58, 95, 205, 255]));
        let scheme = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(scheme.scheme, "tonal_spot");
        assert_ne!(scheme.seed, 0);
        assert!(scheme.roles.contains_key("primary"));
        assert!(scheme.roles.len() >= 30);
        for (role, value) in &scheme.roles {
            assert!(
                value.starts_with('#') && value.len() == 7,
                "role {role} is not #rrggbb: {value}"
            );
        }
    }

    #[test]
    fn identical_covers_generate_identical_schemes() {
        let path = write_cover("twice", |x, y| {
            if (x + y) % 2 == 0 {
                Rgba([200, 60, 90, 255])
            } else {
                Rgba([40, 70, 160, 255])
            }
        });
        let first = generate_from_path(&path, &tonal()).unwrap();
        let second = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(first, second);
    }

    #[test]
    fn grayscale_covers_fall_back_without_error() {
        // No color passes the chroma filter; Score returns its own fallback.
        let path = write_cover("gray", |x, _| {
            let level = (x * 8) as u8;
            Rgba([level, level, level, 255])
        });
        let scheme = generate_from_path(&path, &tonal()).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(scheme.roles.contains_key("surface"));
        assert!(scheme.roles.contains_key("on_surface"));
    }

    #[test]
    fn fully_transparent_covers_are_rejected() {
        let path = write_cover("clear", |_, _| Rgba([10, 20, 30, 0]));
        let result = generate_from_path(&path, &tonal());
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err());
    }

    async fn colors_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations_colors").run(&pool).await.unwrap();
        pool
    }

    async fn covers_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE covers (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn cache_hit_survives_removing_the_source_image() {
        let pool = colors_pool().await;
        let path = write_cover("cache", |_, _| Rgba([58, 95, 205, 255]));

        let first = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default()).await.unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default()).await.unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn changed_cover_hash_regenerates_from_the_image() {
        let pool = colors_pool().await;
        let path = write_cover("stale", |_, _| Rgba([200, 60, 90, 255]));

        fetch_into(&pool, "cover-2", &path, "hash-a", &Options::default()).await.unwrap();
        std::fs::remove_file(&path).unwrap();

        let stale = fetch_into(&pool, "cover-2", &path, "hash-b", &Options::default()).await;
        assert!(stale.is_err(), "a changed hash must regenerate, not reuse");
    }

    #[tokio::test]
    async fn purge_drops_schemes_for_vanished_covers() {
        let colors = colors_pool().await;
        let covers = covers_pool().await;
        sqlx::query("INSERT INTO covers (id) VALUES ('keep')")
            .execute(&covers)
            .await
            .unwrap();

        let path = write_cover("purge", |_, _| Rgba([58, 95, 205, 255]));
        fetch_into(&colors, "keep", &path, "hash", &Options::default()).await.unwrap();
        fetch_into(&colors, "gone", &path, "hash", &Options::default()).await.unwrap();
        let _ = std::fs::remove_file(&path);

        let removed = purge_with(&colors, &covers).await.unwrap();
        assert_eq!(removed, 1);
        assert!(load(&colors, "keep").await.unwrap().is_some());
        assert!(load(&colors, "gone").await.unwrap().is_none());
    }

    #[test]
    fn seed_schemes_are_deterministic_and_serializable() {
        let seed = Rgb::new(58, 95, 205);
        let first = from_seed(seed);
        let second = from_seed(seed);
        assert_eq!(first, second);

        let payload = first.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["scheme"], "tonal_spot");
        assert_eq!(parsed["roles"]["primary"].as_str().unwrap().len(), 7);
        assert_eq!(parsed["seed"].as_u64().unwrap() as u32, seed.as_argb_u32());
    }

    #[test]
    fn smart_variant_thresholds_follow_colorfulness() {
        use material_colors::dynamic_color::Variant;
        assert_eq!(smart_variant(90.0), Variant::Vibrant);
        assert_eq!(smart_variant(40.0), Variant::TonalSpot);
        assert_eq!(smart_variant(10.0), Variant::Neutral);
        assert_eq!(smart_variant(2.0), Variant::Monochrome);
    }

    #[test]
    fn colorfulness_separates_flat_and_loud_pixels() {
        let gray: Vec<Rgb> = (0..254).map(|level| Rgb::new(level, level, level)).collect();
        assert!(colorfulness(&gray) < 6.0, "grays must read as flat");

        let mut loud = Vec::new();
        for _ in 0..128 {
            loud.push(Rgb::new(255, 0, 0));
            loud.push(Rgb::new(0, 0, 255));
        }
        assert!(colorfulness(&loud) > 70.0, "red/blue must read as loud");
    }

    #[test]
    fn knob_changes_invalidate_the_cache_tag() {
        assert_ne!(Options::default().generator(), tonal().generator());
        let dark = Options {
            contrast: -1.0,
            ..Options::default()
        };
        assert_ne!(Options::default().generator(), dark.generator());
    }

    #[test]
    fn contrast_knob_changes_roles() {
        use material_colors::dynamic_color::Variant;
        let seed = Rgb::new(58, 95, 205);
        let standard = scheme_from_seed(seed, Variant::TonalSpot, 0.0);
        let contrast = scheme_from_seed(seed, Variant::TonalSpot, 1.0);
        assert_ne!(standard.roles, contrast.roles);
    }

    #[tokio::test]
    async fn invalidate_forces_regeneration() {
        let pool = colors_pool().await;
        let path = write_cover("invalidate", |_, _| Rgba([58, 95, 205, 255]));
        fetch_into(&pool, "cover-3", &path, "hash", &Options::default())
            .await
            .unwrap();

        invalidate_with(&pool, "cover-3").await.unwrap();
        std::fs::remove_file(&path).unwrap();

        let again = fetch_into(&pool, "cover-3", &path, "hash", &Options::default()).await;
        assert!(again.is_err(), "an invalidated row must regenerate");
    }
}
