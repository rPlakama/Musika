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

//! Dynamic color engine (THM-1..6 / roadmap T0–T5): cover art → Material You
//! dark scheme.
//!
//! The pipeline is split so each stage can be cached and tuned on its own:
//!
//!   `seed`   — decode, downscale, quantize, score (dominant/colorful, DB6
//!              seed cache keyed by cover hash);
//!   `scheme` — seed → roles for the 2021 / 2025 / DMS palette specs;
//!   `engine` — options, DB6 `cover_color` rows, purge and the facade.
//!
//! matugen itself is GPL-2.0; only its engine lineage matters here. We embed
//! the MIT/Apache `material-colors` crate and reimplement the surrounding
//! pipeline, honoring DMS's seed/scheme split (`MATUGENSTUDY.md`).

#![allow(dead_code)] // some milestone APIs (previews/settings) land later.

macro_rules! name_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $text:literal $(| $alias:literal)*,
            )*
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )*
        }

        impl $name {
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)*
                }
            }

            pub fn from_name(value: &str) -> Option<Self> {
                match value.trim().to_ascii_lowercase().as_str() {
                    $($text $(| $alias)* => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

mod engine;
mod rgb;
mod scheme;
mod seed;

#[allow(unused_imports)]
pub use engine::{
    CoverScheme, Options, fetch, fetch_into, from_seed, generate_from_path, hash_roles, invalidate,
    invalidate_with, purge, purge_with,
};
#[allow(unused_imports)]
pub use scheme::{Spec, VariantChoice};
#[allow(unused_imports)]
pub use seed::SourceMode;

#[cfg(test)]
mod tests {
    use super::engine;
    use super::seed;
    use super::*;
    use image::{Rgba, RgbaImage};
    use material_colors::color::Rgb;
    use sqlx::sqlite::SqlitePool;

    /// Writes a synthetic 32x32 cover and returns its path.
    fn write_cover(name: &str, paint: impl Fn(u32, u32) -> Rgba<u8>) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("musika-cover-{}-{name}.png", std::process::id()));
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
            ..Options::default()
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
        assert_eq!(scheme.role_hash.len(), 32);
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

    #[test]
    fn colorful_seed_prefers_the_vivid_region() {
        // A muted blue field with a tiny vivid red corner: dominant scoring
        // follows the field, caelestia scoring the saturated patch.
        let path = write_cover("vivid", |x, y| {
            if x < 4 && y < 4 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([110, 120, 135, 255])
            }
        });
        let (dominant, _) = seed::extract(&path, SourceMode::Dominant).unwrap();
        let (colorful, _) = seed::extract(&path, SourceMode::Colorful).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_ne!(dominant, colorful, "colorful must not equal dominant");
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
    fn knob_changes_invalidate_the_cache_tag() {
        let base = Options::default();
        assert_ne!(base.generator(), tonal().generator());

        let dark = Options {
            contrast: -1.0,
            ..Options::default()
        };
        assert_ne!(base.generator(), dark.generator());

        let colorful = Options {
            source: SourceMode::Colorful,
            ..Options::default()
        };
        assert_ne!(base.generator(), colorful.generator());

        let spec = Options {
            spec: Spec::Spec2021,
            ..Options::default()
        };
        assert_ne!(base.generator(), spec.generator());

        let seeded = Options {
            seed_override: Some(0xFF3A5FCD),
            ..Options::default()
        };
        assert_ne!(base.generator(), seeded.generator());
    }

    #[test]
    fn contrast_knob_changes_roles() {
        let seed = Rgb::new(58, 95, 205);
        let standard = scheme::build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Dms).1;
        let contrast = scheme::build(seed, VariantChoice::TonalSpot, 0.0, 1.0, Spec::Dms).1;
        assert_ne!(standard, contrast);
    }

    async fn colors_pool() -> SqlitePool {
        crate::sqlite::memory_pool(sqlx::migrate!("./migrations_colors")).await
    }

    async fn covers_pool() -> SqlitePool {
        let pool = crate::sqlite::memory_pool(None).await;
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

        let first = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default())
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = fetch_into(&pool, "cover-1", &path, "hash-a", &Options::default())
            .await
            .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn seed_cache_survives_removing_the_source_image() {
        let pool = colors_pool().await;
        let path = write_cover("seed-cache", |_, _| Rgba([58, 95, 205, 255]));

        let first = seed::resolve(&pool, "hash-seed", SourceMode::Colorful, &path, None)
            .await
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        let second = seed::resolve(&pool, "hash-seed", SourceMode::Colorful, &path, None)
            .await
            .unwrap();

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn changed_cover_hash_regenerates_from_the_image() {
        let pool = colors_pool().await;
        let path = write_cover("stale", |_, _| Rgba([200, 60, 90, 255]));

        fetch_into(&pool, "cover-2", &path, "hash-a", &Options::default())
            .await
            .unwrap();
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
        fetch_into(&colors, "keep", &path, "hash", &Options::default())
            .await
            .unwrap();
        fetch_into(&colors, "gone", &path, "hash", &Options::default())
            .await
            .unwrap();
        let _ = std::fs::remove_file(&path);

        let removed = purge_with(&colors, &covers).await.unwrap();
        assert_eq!(removed, 1);
        assert!(engine::load(&colors, "keep").await.unwrap().is_some());
        assert!(engine::load(&colors, "gone").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalidate_regenerates_from_the_cached_seed() {
        let pool = colors_pool().await;
        let path = write_cover("invalidate", |_, _| Rgba([58, 95, 205, 255]));
        let first = fetch_into(&pool, "cover-3", &path, "hash", &Options::default())
            .await
            .unwrap();

        invalidate_with(&pool, "cover-3").await.unwrap();
        assert!(engine::load(&pool, "cover-3").await.unwrap().is_none());
        std::fs::remove_file(&path).unwrap();

        let again = fetch_into(&pool, "cover-3", &path, "hash", &Options::default())
            .await
            .unwrap();
        assert_eq!(first, again, "the seed cache makes regeneration image-free");
    }
}
