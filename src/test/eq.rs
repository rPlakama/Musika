use crate::eq::{EqProfile, FilterKind, profile_path};
use crate::player::mpv::eq::{translate_apo_text, translate_profile};

#[test]
fn parses_parameric_filters_preamp_and_graphic_bands() {
    let profile = EqProfile::parse(
        "Test",
        "Preamp: -6.0 dB\n\
         # comment\n\
         Filter: ON PK Fc 1000 Hz Gain -3.0 dB Q 1.41\n\
         Filter: OFF HP Fc 30 Hz\n\
         GraphicEQ: 20 -3; 100 1.5; 1000 0; 10000 -4.5\n",
    )
    .unwrap();
    assert_eq!(profile.name, "Test");
    assert_eq!(profile.preamp_db, -6.0);
    assert_eq!(profile.filters.len(), 1);
    assert_eq!(profile.filters[0].freq, 1000.0);
    assert!((profile.filters[0].q - 1.41).abs() < 1e-9);
    assert_eq!(profile.graphic_bands.len(), 4);
    assert!((profile.graphic_bands[0].freq - 20.0).abs() < 1e-9);
    assert!((profile.graphic_bands[3].gain_db - (-4.5)).abs() < 1e-9);
}

#[test]
fn parses_autoeq_numbered_filters_bw_oct_lsq_hsq_and_127_band_graphic_eq() {
    let mut text = String::from(
        "Preamp: -5.2 dB\n\
         Stage: pre-mix\n\
         Filter 1: ON LSQ Fc 105 Hz Gain 5.5 dB Q 0.71\n\
         Filter 2: ON PK Fc 250 Hz Gain -2.0 dB BW Oct 1.0\n\
         Filter 3: ON HSQ Fc 10000 Hz Gain -1.5 dB Q 0.71\n\
         GraphicEQ: ",
    );
    for i in 0..127 {
        if i > 0 {
            text.push_str("; ");
        }
        let freq = 20.0 * (1000.0_f64).powf(f64::from(i) / 126.0);
        let gain = if i % 2 == 0 { 1.5 } else { -1.5 };
        text.push_str(&format!("{freq:.1} {gain:.1}"));
    }
    text.push('\n');

    let profile = EqProfile::parse("HD 650", &text).unwrap();
    assert_eq!(profile.filters.len(), 3);
    assert_eq!(profile.filters[0].kind, FilterKind::LowShelf);
    assert_eq!(profile.filters[1].kind, FilterKind::Peak);
    // 1.0 octave bandwidth converts to Q = sqrt(2).
    assert!((profile.filters[1].q - std::f64::consts::SQRT_2).abs() < 1e-3);
    assert_eq!(profile.filters[2].kind, FilterKind::HighShelf);
    assert_eq!(profile.graphic_bands.len(), 127);

    let translated = translate_apo_text("HD 650", &text).unwrap();
    assert!(translated.mpv_af.contains("lowshelf=f=105.000"));
    assert!(translated.mpv_af.contains("highshelf=f=10000.000"));
    assert!(translated.mpv_af.contains("firequalizer=gain_entry='"));
}

#[test]
fn rejects_unknown_directives_and_bad_filters() {
    assert!(EqProfile::parse("x", "Nonsense: 1").is_err());
    assert!(EqProfile::parse("x", "Filter: ON XX Fc 100 Hz").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 0 Hz").is_err());
    assert!(EqProfile::parse("x", "GraphicEQ: 20").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 1000 Hz Gain -3 dB Q").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 1000 Hz stray").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 1000 Hz BW Oct 0").is_err());
}

#[test]
fn profile_path_rejects_traversal() {
    for bad in ["../etc/passwd", "a/b", "..", ".", "a\\b", "", "/etc/passwd"] {
        assert!(
            profile_path(bad).is_err(),
            "profile_path({bad:?}) must return Err"
        );
    }
}

#[test]
fn compiles_preamp_parametric_and_graphic_eq_to_mpv_lavfi() {
    let boost = EqProfile::parse(
        "Boost",
        "Preamp: -3.5 dB\n\
         Filter: ON PK Fc 1000 Hz Gain 6 dB Q 1.0\n\
         GraphicEQ: 60 2.5; 4000 -1.5\n",
    )
    .unwrap();
    let af = translate_profile(&boost);
    assert_eq!(
        af,
        "lavfi=[volume=-3.500dB,equalizer=f=1000.000:width_type=q:w=1.0000:g=6.000,firequalizer=gain_entry='entry(60.000,2.500);entry(4000.000,-1.500)']"
    );

    let preamp_only = EqProfile::parse("Quiet", "Preamp: -6.02 dB").unwrap();
    assert_eq!(translate_profile(&preamp_only), "lavfi=[volume=-6.020dB]");

    let empty = EqProfile::parse("Empty", "# comment only\n").unwrap();
    assert_eq!(translate_profile(&empty), "");
}
