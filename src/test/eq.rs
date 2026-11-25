use crate::eq::{EqProfile, profile_path};

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
    assert_eq!(profile.filters.len(), 5);
    assert_eq!(profile.filters[0].freq, 1000.0);
    assert!((profile.filters[0].q - 1.41).abs() < 1e-9);
    assert!((profile.filters[1].freq - 20.0).abs() < 1e-9);
}

#[test]
fn rejects_unknown_directives_and_bad_filters() {
    assert!(EqProfile::parse("x", "Nonsense: 1").is_err());
    assert!(EqProfile::parse("x", "Filter: ON XX Fc 100 Hz").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 0 Hz").is_err());
    assert!(EqProfile::parse("x", "GraphicEQ: 20").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 1000 Hz Gain -3 dB Q").is_err());
    assert!(EqProfile::parse("x", "Filter: ON PK Fc 1000 Hz stray").is_err());
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
    let af = boost.to_mpv_af();
    assert_eq!(
        af,
        "lavfi=[volume=-3.500dB,equalizer=f=1000.000:width_type=q:w=1.0000:g=6.000,equalizer=f=60.000:width_type=q:w=1.4100:g=2.500,equalizer=f=4000.000:width_type=q:w=1.4100:g=-1.500]"
    );

    let preamp_only = EqProfile::parse("Quiet", "Preamp: -6.02 dB").unwrap();
    assert_eq!(preamp_only.to_mpv_af(), "lavfi=[volume=-6.020dB]");

    let empty = EqProfile::parse("Empty", "# comment only\n").unwrap();
    assert_eq!(empty.to_mpv_af(), "");
}
