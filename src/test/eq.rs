use std::f64::consts::PI;
use std::sync::Arc;

use crate::eq::{
    Biquad, BiquadState, EqFilter, EqProfile, EqRuntime, FilterKind, biquad_for, profile_path,
};

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
fn low_shelf_and_high_shelf_match_rbj_cookbook_dc_and_nyquist_gain() {
    let fs = 48_000.0;
    let f0 = fs / 6.0;
    let gain_db = 12.0;
    let q = std::f64::consts::FRAC_1_SQRT_2;

    let omega = 2.0 * PI * f0 / fs;
    let sin = omega.sin();
    let cos = omega.cos();
    let alpha = sin / (2.0 * q);
    let a = 10f64.powf(gain_db / 40.0);
    let beta = 2.0 * a.sqrt() * alpha;
    let linear_gain = 10f64.powf(gain_db / 20.0);

    let ls = biquad_for(
        &EqFilter {
            kind: FilterKind::LowShelf,
            freq: f0,
            gain_db,
            q,
        },
        fs,
    );
    let ls_a0 = (a + 1.0) + (a - 1.0) * cos + beta;
    let expected_ls = Biquad {
        b0: (a * ((a + 1.0) - (a - 1.0) * cos + beta)) / ls_a0,
        b1: (2.0 * a * ((a - 1.0) - (a + 1.0) * cos)) / ls_a0,
        b2: (a * ((a + 1.0) - (a - 1.0) * cos - beta)) / ls_a0,
        a1: (-2.0 * ((a - 1.0) + (a + 1.0) * cos)) / ls_a0,
        a2: ((a + 1.0) + (a - 1.0) * cos - beta) / ls_a0,
    };
    assert!((ls.b0 - expected_ls.b0).abs() < 1e-12);
    assert!((ls.b1 - expected_ls.b1).abs() < 1e-12);
    assert!((ls.b2 - expected_ls.b2).abs() < 1e-12);
    assert!((ls.a1 - expected_ls.a1).abs() < 1e-12);
    assert!((ls.a2 - expected_ls.a2).abs() < 1e-12);

    let ls_dc = (ls.b0 + ls.b1 + ls.b2) / (1.0 + ls.a1 + ls.a2);
    let ls_nyquist = (ls.b0 - ls.b1 + ls.b2) / (1.0 - ls.a1 + ls.a2);
    assert!((ls_dc - linear_gain).abs() < 1e-12);
    assert!((ls_nyquist - 1.0).abs() < 1e-12);

    let hs = biquad_for(
        &EqFilter {
            kind: FilterKind::HighShelf,
            freq: f0,
            gain_db,
            q,
        },
        fs,
    );
    let hs_a0 = (a + 1.0) - (a - 1.0) * cos + beta;
    let expected_hs = Biquad {
        b0: (a * ((a + 1.0) + (a - 1.0) * cos + beta)) / hs_a0,
        b1: (-2.0 * a * ((a - 1.0) + (a + 1.0) * cos)) / hs_a0,
        b2: (a * ((a + 1.0) + (a - 1.0) * cos - beta)) / hs_a0,
        a1: (2.0 * ((a - 1.0) - (a + 1.0) * cos)) / hs_a0,
        a2: ((a + 1.0) - (a - 1.0) * cos - beta) / hs_a0,
    };
    assert!((hs.b0 - expected_hs.b0).abs() < 1e-12);
    assert!((hs.b1 - expected_hs.b1).abs() < 1e-12);
    assert!((hs.b2 - expected_hs.b2).abs() < 1e-12);
    assert!((hs.a1 - expected_hs.a1).abs() < 1e-12);
    assert!((hs.a2 - expected_hs.a2).abs() < 1e-12);

    let hs_dc = (hs.b0 + hs.b1 + hs.b2) / (1.0 + hs.a1 + hs.a2);
    let hs_nyquist = (hs.b0 - hs.b1 + hs.b2) / (1.0 - hs.a1 + hs.a2);
    assert!((hs_dc - 1.0).abs() < 1e-12);
    assert!((hs_nyquist - linear_gain).abs() < 1e-12);

    let mut denorm_state = BiquadState::default();
    assert_eq!(denorm_state.process(&ls, 1e-20), 0.0);
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
fn preamp_and_peak_filter_change_the_signal_level() {
    let profile = Arc::new(
        EqProfile::parse(
            "Boost",
            "Preamp: 0 dB\nFilter: ON PK Fc 1000 Hz Gain 6 dB Q 1.0\n",
        )
        .unwrap(),
    );
    let mut runtime = EqRuntime::new(profile, 48_000, 2);
    let frames = 24_000usize;
    let mut pcm = vec![0.0f32; frames * 2];
    for frame in 0..frames {
        let sample = (0.1 * (2.0 * PI * 1000.0 * frame as f64 / 48_000.0).sin()) as f32;
        pcm[frame * 2] = sample;
        pcm[frame * 2 + 1] = sample;
    }
    let input_rms = rms(&pcm);
    runtime.process(&mut pcm, frames);
    let output_rms = rms(&pcm);
    assert!(
        output_rms > input_rms * 1.5,
        "a +6 dB peak at the tone frequency must boost it: {input_rms} -> {output_rms}"
    );
}

#[test]
fn preamp_only_profile_scales_by_the_linear_gain() {
    let profile = Arc::new(EqProfile::parse("Quiet", "Preamp: -6.02 dB").unwrap());
    let mut runtime = EqRuntime::new(profile, 48_000, 1);
    let mut pcm = vec![1.0f32; 64];
    runtime.process(&mut pcm, 64);
    // Reaches the steady state after a few samples; the tail is exact.
    assert!((pcm[63] - 0.5).abs() < 0.01, "{}", pcm[63]);
}

fn rms(pcm: &[f32]) -> f64 {
    let sum: f64 = pcm.iter().map(|sample| f64::from(*sample).powi(2)).sum();
    (sum / pcm.len() as f64).sqrt()
}
