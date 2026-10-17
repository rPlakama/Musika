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

//! Palette generation (THM-6 R2): seed → Material roles.
//!
//! Specs: `2021` (the crate default, what matugen emits), `2025` (Material's
//! 2025 spec, in-crate), `dms` (2025 with DMS's raised accent-chroma tables and
//! neutral surfaces, ported from DMS `spec.go`). Role resolution always runs
//! through the crate's `DynamicScheme`; only the palettes differ.

use std::collections::BTreeMap;

use material_colors::color::Rgb;
use material_colors::dynamic_color::{DynamicScheme, Platform, SpecVersion, Variant};
use material_colors::hct::Hct;
use material_colors::palette::TonalPalette;

name_enum! {
    /// Palette spec versions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum Spec {
        Spec2021 => "2021" | "spec2021",
        Spec2025 => "2025" | "spec2025",
        /// The default: DMS parity (2025 + raised accents).
        #[default]
        Dms => "dms",
    }
}

name_enum! {
    /// Which scheme family to build. `Smart` picks per image (T4).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum VariantChoice {
        /// Choose by Hasler-Süsstrunk colorfulness (the default).
        #[default]
        Smart => "smart",
        TonalSpot => "tonal_spot",
        Vibrant => "vibrant",
        Expressive => "expressive",
        Neutral => "neutral",
        Monochrome => "monochrome",
        Fidelity => "fidelity",
        Content => "content",
        Rainbow => "rainbow",
        FruitSalad => "fruit_salad",
    }
}

impl VariantChoice {
    fn from_variant(variant: Variant) -> Option<Self> {
        match variant {
            Variant::TonalSpot => Some(Self::TonalSpot),
            Variant::Vibrant => Some(Self::Vibrant),
            Variant::Expressive => Some(Self::Expressive),
            Variant::Neutral => Some(Self::Neutral),
            Variant::Monochrome => Some(Self::Monochrome),
            Variant::Fidelity => Some(Self::Fidelity),
            Variant::Content => Some(Self::Content),
            Variant::Rainbow => Some(Self::Rainbow),
            Variant::FruitSalad => Some(Self::FruitSalad),
            Variant::Cmf => None,
        }
    }

    /// Resolves the concrete scheme for an image's colorfulness.
    pub fn resolve(self, colorfulness: f64) -> Variant {
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
pub fn smart_variant(colorfulness: f64) -> Variant {
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

/// Builds the dark scheme for a seed and returns its variant name plus the
/// curated role map.
pub fn build(
    seed: Rgb,
    variant: VariantChoice,
    colorfulness: f64,
    contrast: f64,
    spec: Spec,
) -> (String, BTreeMap<String, String>) {
    let variant = variant.resolve(colorfulness);
    let dynamic = dynamic_scheme(seed, variant, spec, contrast, true);
    (variant_name(&variant).to_string(), role_map(&dynamic))
}

/// Builds the `DynamicScheme` for one spec.
pub fn dynamic_scheme(
    seed: Rgb,
    variant: Variant,
    spec: Spec,
    contrast: f64,
    is_dark: bool,
) -> DynamicScheme {
    let hct: Hct = seed.into();
    match spec {
        Spec::Spec2021 => spec_scheme(hct, variant, SpecVersion::Spec2021, contrast, is_dark),
        Spec::Spec2025 => spec_scheme(hct, variant, SpecVersion::Spec2025, contrast, is_dark),
        Spec::Dms => dms_scheme(hct, variant, contrast, is_dark),
    }
}

fn spec_scheme(
    hct: Hct,
    variant: Variant,
    version: SpecVersion,
    contrast: f64,
    is_dark: bool,
) -> DynamicScheme {
    DynamicScheme::from_spec(
        hct,
        variant,
        is_dark,
        Some(contrast),
        Platform::Phone,
        version,
    )
}

/// DMS parity (`newDMSScheme` in `spec.go`): 2025 palettes with raised accent
/// chromas, and neutral surfaces always taken from Tonal Spot. Tonal Spot runs
/// Vibrant's tone rules; Expressive keeps its 2021 rotated hues.
fn dms_scheme(hct: Hct, variant: Variant, contrast: f64, is_dark: bool) -> DynamicScheme {
    let neutral = spec_scheme(
        hct,
        Variant::TonalSpot,
        SpecVersion::Spec2025,
        contrast,
        is_dark,
    );

    let (rules, chromas) = match variant {
        Variant::TonalSpot => (Variant::Vibrant, (40.0, 28.0, 36.0)),
        Variant::Expressive => (Variant::TonalSpot, (48.0, 40.0, 48.0)),
        Variant::Neutral => (Variant::Neutral, (20.0, 12.0, 28.0)),
        _ => {
            // Vibrant and anything the 2025 spec does not define: keep the
            // variant's own palettes, only the neutral surfaces come from
            // Tonal Spot.
            let mut scheme = spec_scheme(hct, variant, SpecVersion::Spec2025, contrast, is_dark);
            scheme.neutral_palette = neutral.neutral_palette;
            scheme.neutral_variant_palette = neutral.neutral_variant_palette;
            return scheme;
        }
    };

    let own = spec_scheme(hct, variant, SpecVersion::Spec2025, contrast, is_dark);
    let (secondary_hue, tertiary_hue) = if variant == Variant::Expressive {
        // 2025 Expressive dropped the rotated secondary/tertiary; they are its
        // identity, so take the hues from 2021.
        let old = spec_scheme(hct, variant, SpecVersion::Spec2021, contrast, is_dark);
        (old.secondary_palette.hue(), old.tertiary_palette.hue())
    } else {
        (own.secondary_palette.hue(), own.tertiary_palette.hue())
    };

    let mut scheme = spec_scheme(hct, rules, SpecVersion::Spec2025, contrast, is_dark);
    scheme.primary_palette = TonalPalette::of(own.primary_palette.hue(), chromas.0);
    scheme.secondary_palette = TonalPalette::of(secondary_hue, chromas.1);
    scheme.tertiary_palette = TonalPalette::of(tertiary_hue, chromas.2);
    scheme.neutral_palette = neutral.neutral_palette;
    scheme.neutral_variant_palette = neutral.neutral_variant_palette;
    scheme
}

fn variant_name(variant: &Variant) -> &'static str {
    VariantChoice::from_variant(*variant).map_or("cmf", VariantChoice::name)
}

/// The curated role subset: everything `Palette` exposes, and nothing
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
    put(
        "surface_container_lowest",
        dynamic.surface_container_lowest(),
    );
    put("surface_container_low", dynamic.surface_container_low());
    put("surface_container", dynamic.surface_container());
    put("surface_container_high", dynamic.surface_container_high());
    put(
        "surface_container_highest",
        dynamic.surface_container_highest(),
    );
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
    super::rgb::hex(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smart_thresholds_follow_colorfulness() {
        assert_eq!(smart_variant(90.0), Variant::Vibrant);
        assert_eq!(smart_variant(40.0), Variant::TonalSpot);
        assert_eq!(smart_variant(10.0), Variant::Neutral);
        assert_eq!(smart_variant(2.0), Variant::Monochrome);
    }

    #[test]
    fn specs_produce_different_roles() {
        let seed = Rgb::new(58, 95, 205);
        let (_, roles_2021) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Spec2021);
        let (_, roles_2025) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Spec2025);
        let (_, roles_dms) = build(seed, VariantChoice::TonalSpot, 0.0, 0.0, Spec::Dms);
        assert_ne!(roles_2021, roles_2025, "2021 and 2025 must differ");
        assert_ne!(roles_2025, roles_dms, "DMS raises the accent chroma");
    }

    #[test]
    fn dms_scheme_keeps_neutral_surfaces_from_tonal_spot() {
        let seed = Rgb::new(200, 60, 90);
        let dms = dynamic_scheme(seed, Variant::Vibrant, Spec::Dms, 0.0, true);
        let hct: Hct = seed.into();
        let neutral = DynamicScheme::from_spec(
            hct,
            Variant::TonalSpot,
            true,
            Some(0.0),
            Platform::Phone,
            SpecVersion::Spec2025,
        );
        assert_eq!(dms.neutral_palette.hue(), neutral.neutral_palette.hue());
        assert_eq!(
            dms.neutral_palette.chroma(),
            neutral.neutral_palette.chroma()
        );
    }

    #[test]
    fn all_roles_are_hex() {
        let (name, roles) = build(
            Rgb::new(58, 95, 205),
            VariantChoice::TonalSpot,
            0.0,
            0.0,
            Spec::Dms,
        );
        assert_eq!(name, "tonal_spot");
        assert!(roles.len() >= 30);
        for value in roles.values() {
            assert!(value.starts_with('#') && value.len() == 7, "{value}");
        }
    }
}
