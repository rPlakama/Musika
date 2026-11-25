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

//! Equalizer APO → MPV `lavfi` translation layer.
//!
//! Invoked by the daemon/IPC layer so APO profiles are parsed and translated
//! once into MPV's `af` filter-graph syntax before reaching the playback
//! engine, and so IPC queries (`eq list`) can expose the exact live MPV filter.

use anyhow::Result;

use crate::eq::{EqFilter, EqProfile, FilterKind, GraphicBand};

/// One Equalizer APO profile translated into an MPV `af` property value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranslatedEq {
    pub name: String,
    pub mpv_af: String,
}

/// Loads a named Equalizer APO profile from disk and translates it to MPV's
/// `lavfi=[...]` audio filter format.
pub(crate) fn load_and_translate(name: &str) -> Result<TranslatedEq> {
    let profile = crate::eq::load_profile(name)?;
    Ok(TranslatedEq {
        mpv_af: translate_profile(&profile),
        name: profile.name,
    })
}

/// Parses raw Equalizer APO text and translates it to MPV's `lavfi=[...]`
/// format in one step.
#[cfg(test)]
pub(crate) fn translate_apo_text(name: &str, text: &str) -> Result<TranslatedEq> {
    let profile = EqProfile::parse(name, text)?;
    Ok(TranslatedEq {
        mpv_af: translate_profile(&profile),
        name: profile.name,
    })
}

/// Translates a parsed [`EqProfile`] into an MPV `af` property value using
/// FFmpeg's `lavfi` filters (`lavfi=[...]`).
///
/// - `Preamp:` → `volume=<dB>dB`
/// - Parametric `Filter:` → `equalizer` / `lowshelf` / `highshelf` / `lowpass`
///   / `highpass` / `bandpass` / `bandreject`
/// - `GraphicEQ:` → `firequalizer=gain_entry='entry(f1,g1);...'` (exact
///   piecewise-linear response matching Equalizer APO / AutoEq).
///
/// Returns an empty string when the profile is a no-op (zero preamp, no active
/// parametric filters, and no non-zero GraphicEQ bands).
pub(crate) fn translate_profile(profile: &EqProfile) -> String {
    let mut stages = Vec::with_capacity(profile.filters.len() + 2);
    if profile.preamp_db.abs() >= 1e-4 {
        stages.push(format!("volume={:.3}dB", profile.preamp_db));
    }
    for filter in &profile.filters {
        stages.push(translate_filter(filter));
    }
    if let Some(graphic) = translate_graphic_bands(&profile.graphic_bands) {
        stages.push(graphic);
    }
    if stages.is_empty() {
        String::new()
    } else {
        format!("lavfi=[{}]", stages.join(","))
    }
}

fn translate_filter(filter: &EqFilter) -> String {
    match filter.kind {
        FilterKind::Peak => format!(
            "equalizer=f={:.3}:width_type=q:w={:.4}:g={:.3}",
            filter.freq, filter.q, filter.gain_db
        ),
        FilterKind::LowShelf => format!(
            "lowshelf=f={:.3}:width_type=q:w={:.4}:g={:.3}",
            filter.freq, filter.q, filter.gain_db
        ),
        FilterKind::HighShelf => format!(
            "highshelf=f={:.3}:width_type=q:w={:.4}:g={:.3}",
            filter.freq, filter.q, filter.gain_db
        ),
        FilterKind::LowPass => format!(
            "lowpass=f={:.3}:width_type=q:w={:.4}",
            filter.freq, filter.q
        ),
        FilterKind::HighPass => format!(
            "highpass=f={:.3}:width_type=q:w={:.4}",
            filter.freq, filter.q
        ),
        FilterKind::BandPass => format!(
            "bandpass=f={:.3}:width_type=q:w={:.4}",
            filter.freq, filter.q
        ),
        FilterKind::Notch => format!(
            "bandreject=f={:.3}:width_type=q:w={:.4}",
            filter.freq, filter.q
        ),
    }
}

fn translate_graphic_bands(bands: &[GraphicBand]) -> Option<String> {
    if bands.is_empty() || bands.iter().all(|band| band.gain_db.abs() < 1e-4) {
        return None;
    }
    let entries: Vec<String> = bands
        .iter()
        .map(|band| format!("entry({:.3},{:.3})", band.freq, band.gain_db))
        .collect();
    Some(format!("firequalizer=gain_entry='{}'", entries.join(";")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_preamp_all_parametric_filters_and_piecewise_graphic_eq() {
        let translated = translate_apo_text(
            "Studio",
            "Preamp: -4.5 dB\n\
             Filter 1: ON PK Fc 1000 Hz Gain 3.0 dB Q 1.41\n\
             Filter 2: ON LS Fc 80 Hz Gain 4.0 dB Q 0.707\n\
             Filter 3: ON HS Fc 8000 Hz Gain -2.0 dB Q 0.707\n\
             Filter 4: ON LP Fc 16000 Hz Q 0.707\n\
             Filter 5: ON HP Fc 30 Hz Q 0.707\n\
             Filter 6: ON BP Fc 500 Hz Q 2.0\n\
             Filter 7: ON NO Fc 60 Hz Q 8.0\n\
             GraphicEQ: 20 -3; 100 1.5; 10000 -4.5\n",
        )
        .expect("translate apo");

        let af = &translated.mpv_af;
        assert!(af.starts_with("lavfi=[") && af.ends_with(']'), "af={af}");
        assert!(af.contains("volume=-4.500dB"), "af={af}");
        assert!(af.contains("equalizer=f=1000.000:width_type=q:w=1.4100:g=3.000"));
        assert!(af.contains("lowshelf=f=80.000:width_type=q:w=0.7070:g=4.000"));
        assert!(af.contains("highshelf=f=8000.000:width_type=q:w=0.7070:g=-2.000"));
        assert!(af.contains("lowpass=f=16000.000:width_type=q:w=0.7070"));
        assert!(af.contains("highpass=f=30.000:width_type=q:w=0.7070"));
        assert!(af.contains("bandpass=f=500.000:width_type=q:w=2.0000"));
        assert!(af.contains("bandreject=f=60.000:width_type=q:w=8.0000"));
        assert!(af.contains(
            "firequalizer=gain_entry='entry(20.000,-3.000);entry(100.000,1.500);entry(10000.000,-4.500)'"
        ));
    }

    #[test]
    fn flat_or_zero_graphic_profile_translates_to_empty_bypass() {
        let flat = translate_apo_text(
            "Flat",
            "Preamp: 0 dB\nFilter: OFF PK Fc 1000 Hz Gain 3 dB\nGraphicEQ: 20 0; 1000 0\n",
        )
        .expect("translate flat");
        assert!(flat.mpv_af.is_empty());
    }
}
