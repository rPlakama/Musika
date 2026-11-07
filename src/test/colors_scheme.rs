use super::common::*;
use crate::colors::scheme::*;

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
