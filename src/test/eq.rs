use std::f64::consts::PI;
use std::sync::Arc;

use crate::eq::{EqProfile, EqRuntime};

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
