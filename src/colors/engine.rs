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

//! Cache-aware orchestration (THM-6 R0/R3): options, DB6 rows, purge.
//!
//! `fetch` resolves cover → seed (cached) → palette (cheap) and stores the
//! result keyed by cover id, revalidated against `covers.hash` and the full
//! `generator` tag (spec, variant, source mode, contrast, seed override).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use material_colors::color::Rgb;
use sqlx::FromRow;
use sqlx::sqlite::SqlitePool;

use super::scheme::{self, Spec, VariantChoice};
use super::seed::{self, SourceMode};
use crate::consts::now_secs;
use crate::db::Dbs;

/// Generation knobs. Anything that changes the role map must be part of the
/// `generator()` tag so stored schemes invalidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub variant: VariantChoice,
    pub contrast: f64,
    pub source: SourceMode,
    pub spec: Spec,
    /// Explicit `#rrggbb` seed; skips the image entirely.
    pub seed_override: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            variant: VariantChoice::Smart,
            contrast: 0.0,
            source: SourceMode::Dominant,
            spec: Spec::Dms,
            seed_override: None,
        }
    }
}

impl Options {
    /// Reads `MUSIKA_COLOR_VARIANT` / `_CONTRAST` / `_SOURCE` / `_SPEC`.
    /// (`_SOURCE` accepts `dominant`, `colorful` or a `#rrggbb` seed.)
    pub fn from_env() -> Self {
        let contrast = std::env::var("MUSIKA_COLOR_CONTRAST")
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .filter(|value: &f64| (-1.0..=1.0).contains(value))
            .unwrap_or(0.0);

        let mut source = SourceMode::Dominant;
        let mut seed_override = None;
        if let Ok(value) = std::env::var("MUSIKA_COLOR_SOURCE") {
            let value = value.trim();
            if let Some(mode) = SourceMode::from_name(value) {
                source = mode;
            } else if let Some(argb) = super::rgb::parse_hex(value) {
                seed_override = Some(argb);
            }
        }

        let spec = std::env::var("MUSIKA_COLOR_SPEC")
            .ok()
            .and_then(|value| Spec::from_name(&value))
            .unwrap_or_default();

        Self {
            variant: std::env::var("MUSIKA_COLOR_VARIANT")
                .ok()
                .and_then(|value| VariantChoice::from_name(&value))
                .unwrap_or_default(),
            contrast,
            source,
            spec,
            seed_override,
        }
    }

    /// Cache tag: any knob change invalidates stored schemes.
    pub fn generator(&self) -> String {
        format!(
            "musika-colors/{}/{}/{}/{:.2}/{}",
            self.spec.name(),
            self.variant.name(),
            self.source.name(),
            self.contrast,
            self.seed_override
                .map(|argb| format!("{argb:08x}"))
                .unwrap_or_else(|| "-".to_string()),
        )
    }
}

/// A generated dark scheme for one cover.
#[derive(Debug, Clone, PartialEq)]
pub struct CoverScheme {
    /// Variant name (`tonal_spot`, `vibrant`, …).
    pub scheme: String,
    /// ARGB of the scored source color.
    pub seed: u32,
    /// Image colorfulness that drove the `smart` variant.
    pub colorfulness: f64,
    /// Hash of `roles`; identical palettes are a no-op for the UI.
    pub role_hash: String,
    /// Role name -> `#rrggbb`.
    pub roles: BTreeMap<String, String>,
}

impl CoverScheme {
    /// Payload used by the cache and the `colors` dataReady channel.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "scheme": self.scheme,
            "seed": self.seed,
            "colorfulness": self.colorfulness,
            "role_hash": self.role_hash,
            "roles": self.roles,
        })
        .to_string()
    }
}

/// Returns the scheme for a cached cover, generating and storing it when the
/// row is missing or stale. `None` when the cover itself is not cached (no art
/// → static palette).
///
/// The decoder is handed the cached thumbnail, not the original: the seed is
/// sampled at 112 px, so the extra resolution is only peak memory and CPU.
pub async fn fetch(dbs: &Dbs, cover_art: &str) -> Result<Option<CoverScheme>> {
    let Some(path) = crate::covers::palette_path(&dbs.covers, cover_art).await? else {
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
                colorfulness: row.colorfulness,
                role_hash: row.role_hash,
                roles,
            });
        }
    }

    let resolved = seed::resolve(
        pool,
        cover_hash,
        options.source,
        path,
        options.seed_override,
    )
    .await?;

    let knobs = *options;
    let colorfulness = resolved.colorfulness;
    let scheme =
        tokio::task::spawn_blocking(move || build_scheme(resolved.color, colorfulness, &knobs))
            .await
            .context("palette task panicked")??;

    store(pool, cover_art, cover_hash, &generator, &scheme).await?;
    Ok(scheme)
}

/// Synchronous generation without the DB (tests, previews).
pub fn generate_from_path(path: &Path, options: &Options) -> Result<CoverScheme> {
    let (color, colorfulness) = match options.seed_override {
        Some(argb) => (super::rgb::from_argb(argb), 0.0),
        None => seed::extract(path, options.source)?,
    };
    Ok(build_scheme(color, colorfulness, options)?)
}

/// Convenience wrapper: Tonal Spot at standard contrast under the default
/// spec (used by tests and any caller that wants a plain scheme).
pub fn from_seed(seed: Rgb) -> Result<CoverScheme> {
    build_scheme(
        seed,
        0.0,
        &Options {
            variant: VariantChoice::TonalSpot,
            ..Options::default()
        },
    )
}

fn build_scheme(seed: Rgb, colorfulness: f64, options: &Options) -> Result<CoverScheme> {
    let (name, roles) = scheme::build(
        seed,
        options.variant,
        colorfulness,
        options.contrast,
        options.spec,
    );
    Ok(CoverScheme {
        scheme: name,
        seed: super::rgb::argb(seed),
        colorfulness,
        role_hash: hash_roles(&roles)?,
        roles,
    })
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

#[derive(Debug, FromRow)]
pub(crate) struct CachedRow {
    cover_hash: String,
    scheme: String,
    seed: i64,
    colorfulness: f64,
    dark: String,
    generator: String,
    role_hash: String,
}

pub(crate) async fn load(pool: &SqlitePool, cover_art: &str) -> Result<Option<CachedRow>> {
    let row = sqlx::query_as::<_, CachedRow>(
        "SELECT cover_hash, scheme, seed, colorfulness, dark, generator, role_hash
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
        "INSERT INTO cover_color
             (cover_art, cover_hash, scheme, seed, colorfulness, dark, generator, role_hash, generated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(cover_art) DO UPDATE SET
             cover_hash = excluded.cover_hash,
             scheme = excluded.scheme,
             seed = excluded.seed,
             colorfulness = excluded.colorfulness,
             dark = excluded.dark,
             generator = excluded.generator,
             role_hash = excluded.role_hash,
             generated_at = excluded.generated_at",
    )
    .bind(cover_art)
    .bind(cover_hash)
    .bind(&scheme.scheme)
    .bind(scheme.seed as i64)
    .bind(scheme.colorfulness)
    .bind(dark)
    .bind(generator)
    .bind(&scheme.role_hash)
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes cached schemes whose cover id vanished from covers.db.
pub async fn purge(dbs: &Dbs) -> Result<usize> {
    purge_with(&dbs.colors, &dbs.covers).await
}

pub async fn purge_with(colors: &SqlitePool, covers: &SqlitePool) -> Result<usize> {
    let existing: std::collections::HashSet<String> = sqlx::query_scalar("SELECT id FROM covers")
        .fetch_all(covers)
        .await?
        .into_iter()
        .collect();
    crate::cache::purge_missing(colors, "cover_color", "cover_art", &existing).await
}

/// Canonical hash of a role map (BTreeMap → stable JSON → md5). The UI treats
/// an unchanged hash as "no work".
pub fn hash_roles(roles: &BTreeMap<String, String>) -> Result<String> {
    let canonical = serde_json::to_string(roles)?;
    crate::digest::md5_hex(canonical.as_bytes())
}
