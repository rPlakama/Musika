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

//! Equalizer APO–style profiles.
//!
//! Profiles live in `$XDG_CONFIG_HOME/musika/eqs/*.{txt,eq}` and are a safe
//! subset of Equalizer APO's text format: a `Preamp:` line, parametric
//! `Filter:` lines (PK/PEQ, LS, HS, LP, HP, BP, NO), and `GraphicEQ:` bands.
//! Parsing is deliberately strict: a profile either produces a deterministic
//! biquad chain or is rejected with the line number, never silently skipped.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const MAX_FILTERS: usize = 64;
const MAX_GRAPHIC_BANDS: usize = 31;
const MAX_GAIN_DB: f64 = 24.0;
const MIN_Q: f64 = 0.05;
const MAX_Q: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Peak,
    LowShelf,
    HighShelf,
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqFilter {
    pub kind: FilterKind,
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqProfile {
    pub name: String,
    pub preamp_db: f64,
    pub filters: Vec<EqFilter>,
}

impl EqProfile {
    /// Parses one Equalizer APO–style profile. `name` is the display name
    /// (normally the file stem).
    pub fn parse(name: &str, text: &str) -> Result<Self> {
        let mut preamp_db = 0.0f64;
        let mut filters = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (directive, rest) = line
                .split_once(':')
                .with_context(|| format!("line {}: expected 'Directive: value'", line_no + 1))?;
            match directive.trim().to_ascii_lowercase().as_str() {
                "preamp" => {
                    preamp_db = parse_first_number(rest)
                        .with_context(|| format!("line {}: preamp", line_no + 1))?
                        .clamp(-60.0, MAX_GAIN_DB);
                }
                "filter" => {
                    if let Some(filter) =
                        parse_filter(rest).with_context(|| format!("line {}", line_no + 1))?
                    {
                        filters.push(filter);
                    }
                }
                "graphiceq" => {
                    let bands =
                        parse_graphic(rest).with_context(|| format!("line {}", line_no + 1))?;
                    filters.extend(bands);
                }
                // Equalizer APO may scope a file to a device/channel; Musika
                // applies the profile to the stereo downmix, so these are
                // accepted and ignored.
                "device" | "channel" => {}
                other => bail!("line {}: unsupported directive '{other}'", line_no + 1),
            }
            if filters.len() > MAX_FILTERS {
                bail!("too many filters (max {MAX_FILTERS})");
            }
        }
        Ok(Self {
            name: name.to_string(),
            preamp_db,
            filters,
        })
    }

    pub fn gain(&self) -> f32 {
        10f64.powf(self.preamp_db / 20.0) as f32
    }
}

/// `ON PK Fc 1000 Hz Gain -3 dB Q 1.4`; an OFF line is a valid no-op.
fn parse_filter(rest: &str) -> Result<Option<EqFilter>> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let Some(on) = tokens.first() else {
        bail!("empty Filter");
    };
    let enabled = match on.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        other => bail!("Filter must start with ON/OFF, got '{other}'"),
    };
    let Some(kind_token) = tokens.get(1) else {
        bail!("Filter type is missing");
    };
    let kind = match kind_token.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" => FilterKind::Peak,
        "LS" | "LSC" => FilterKind::LowShelf,
        "HS" | "HSC" => FilterKind::HighShelf,
        "LP" | "LPQ" => FilterKind::LowPass,
        "HP" | "HPQ" => FilterKind::HighPass,
        "BP" => FilterKind::BandPass,
        "NO" | "NOTCH" => FilterKind::Notch,
        other => bail!("unsupported Filter type '{other}'"),
    };
    let mut freq = 1000.0f64;
    let mut gain_db = 0.0f64;
    let mut q = 0.707f64;
    let mut index = 2;
    while index + 1 < tokens.len() {
        match tokens[index].to_ascii_lowercase().as_str() {
            "fc" => {
                freq = tokens[index + 1]
                    .parse()
                    .with_context(|| format!("invalid Fc '{}'", tokens[index + 1]))?;
                index += 2;
                if tokens
                    .get(index)
                    .is_some_and(|unit| unit.eq_ignore_ascii_case("hz"))
                {
                    index += 1;
                }
            }
            "gain" => {
                gain_db = tokens[index + 1]
                    .parse()
                    .with_context(|| format!("invalid Gain '{}'", tokens[index + 1]))?;
                index += 2;
                if tokens
                    .get(index)
                    .is_some_and(|unit| unit.eq_ignore_ascii_case("db"))
                {
                    index += 1;
                }
            }
            "q" => {
                q = tokens[index + 1]
                    .parse()
                    .with_context(|| format!("invalid Q '{}'", tokens[index + 1]))?;
                index += 2;
            }
            other => bail!("unexpected Filter token '{other}'"),
        }
    }
    if index < tokens.len() {
        bail!(
            "unexpected trailing token '{}' in Filter line",
            tokens[index]
        );
    }
    if !enabled {
        return Ok(None);
    }
    if !(1.0..=48_000.0).contains(&freq) {
        bail!("Fc {freq} Hz is outside 1..=48000");
    }
    if !(-MAX_GAIN_DB..=MAX_GAIN_DB).contains(&gain_db) {
        bail!("Gain {gain_db} dB is outside ±{MAX_GAIN_DB}");
    }
    Ok(Some(EqFilter {
        kind,
        freq,
        gain_db: gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
        q: q.clamp(MIN_Q, MAX_Q),
    }))
}

/// `20 -3; 25 -3; ...` as peaking bands with an octave-scale Q.
fn parse_graphic(rest: &str) -> Result<Vec<EqFilter>> {
    let mut filters = Vec::new();
    for pair in rest.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.split_whitespace();
        let freq: f64 = parts
            .next()
            .context("GraphicEQ band is missing its frequency")?
            .parse()
            .with_context(|| format!("invalid GraphicEQ frequency '{pair}'"))?;
        let gain: f64 = parts
            .next()
            .context("GraphicEQ band is missing its gain")?
            .parse()
            .with_context(|| format!("invalid GraphicEQ gain '{pair}'"))?;
        if !(1.0..=48_000.0).contains(&freq) {
            bail!("GraphicEQ frequency {freq} Hz is outside 1..=48000");
        }
        filters.push(EqFilter {
            kind: FilterKind::Peak,
            freq,
            gain_db: gain.clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
            // A one-octave peaking band is the standard graphic-EQ
            // approximation; the parser caps the band count.
            q: 1.41,
        });
        if filters.len() > MAX_GRAPHIC_BANDS {
            bail!("too many GraphicEQ bands (max {MAX_GRAPHIC_BANDS})");
        }
    }
    Ok(filters)
}

fn parse_first_number(text: &str) -> Result<f64> {
    text.split_whitespace()
        .next()
        .context("expected a number")?
        .parse()
        .context("invalid number")
}

/// Sorted profile names (file stems) from the account's `eqs` directory.
pub fn list_profiles() -> Vec<String> {
    let Ok(dir) = profiles_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() || !is_profile_path(&path) {
                return None;
            }
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    names.dedup();
    names
}

pub fn load_profile(name: &str) -> Result<EqProfile> {
    let path = profile_path(name).with_context(|| format!("profile '{name}' not found"))?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    EqProfile::parse(name, &text)
}

pub fn profile_path(name: &str) -> Result<PathBuf> {
    validate_profile_name(name)?;
    let dir = profiles_dir()?;
    for extension in ["txt", "eq"] {
        let path = dir.join(format!("{name}.{extension}"));
        if path.parent() == Some(dir.as_path()) && path.is_file() {
            return Ok(path);
        }
    }
    bail!("no '{name}.txt' or '{name}.eq' in {}", dir.display())
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || name.chars().any(std::path::is_separator)
    {
        bail!("invalid EQ profile name '{name}'");
    }
    let mut components = Path::new(name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        bail!("invalid EQ profile name '{name}'");
    }
    Ok(())
}

pub fn profiles_dir() -> Result<PathBuf> {
    Ok(crate::db::config_dir()?.join("eqs"))
}

fn is_profile_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("txt") || extension.eq_ignore_ascii_case("eq")
        })
}

/// Normalized biquad coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Biquad {
    pub(crate) b0: f64,
    pub(crate) b1: f64,
    pub(crate) b2: f64,
    pub(crate) a1: f64,
    pub(crate) a2: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BiquadState {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadState {
    pub(crate) fn process(&mut self, biquad: &Biquad, x: f64) -> f64 {
        let y = biquad.b0 * x + biquad.b1 * self.x1 + biquad.b2 * self.x2
            - biquad.a1 * self.y1
            - biquad.a2 * self.y2;
        let y = if y.abs() < 1e-18 { 0.0 } else { y };
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A profile compiled for one sample rate and channel count.
#[derive(Debug, Clone)]
pub struct EqRuntime {
    preamp: f32,
    biquads: Vec<Biquad>,
    states: Vec<Vec<BiquadState>>,
    channels: usize,
}

impl EqRuntime {
    pub fn new(profile: std::sync::Arc<EqProfile>, sample_rate: u32, channels: usize) -> Self {
        let preamp = profile.gain();
        let biquads = profile
            .filters
            .iter()
            .map(|filter| biquad_for(filter, f64::from(sample_rate.max(1))))
            .collect();
        let states = vec![vec![BiquadState::default(); channels]; profile.filters.len()];
        Self {
            preamp,
            biquads,
            states,
            channels,
        }
    }

    /// Applies the chain to interleaved F32 PCM.
    pub fn process(&mut self, pcm: &mut [f32], frames: usize) {
        if self.biquads.is_empty() && (self.preamp - 1.0).abs() < 1e-6 {
            return;
        }
        let channels = self.channels.max(1);
        let frames = frames.min(pcm.len() / channels);
        let preamp = f64::from(self.preamp);
        for frame in 0..frames {
            for channel in 0..channels {
                let index = frame * channels + channel;
                let mut sample = f64::from(pcm[index]) * preamp;
                for (biquad, states) in self.biquads.iter().zip(self.states.iter_mut()) {
                    sample = states[channel].process(biquad, sample);
                }
                pcm[index] = sample as f32;
            }
        }
    }
}

pub(crate) fn biquad_for(filter: &EqFilter, sample_rate: f64) -> Biquad {
    let freq = filter.freq.clamp(1.0, sample_rate * 0.49);
    let omega = 2.0 * PI * freq / sample_rate;
    let sin = omega.sin();
    let cos = omega.cos();
    let q = filter.q.max(MIN_Q);
    let alpha = sin / (2.0 * q);
    let a = 10f64.powf(filter.gain_db / 40.0);
    let (mut b0, mut b1, mut b2, a0, mut a1, mut a2) = match filter.kind {
        FilterKind::Peak => (
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ),
        FilterKind::LowShelf => {
            let beta = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * cos + beta),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - beta),
                (a + 1.0) + (a - 1.0) * cos + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - beta,
            )
        }
        FilterKind::HighShelf => {
            let beta = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * cos + beta),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - beta),
                (a + 1.0) - (a - 1.0) * cos + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - beta,
            )
        }
        FilterKind::LowPass => (
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterKind::HighPass => (
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        FilterKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
    };
    if a0.abs() < f64::EPSILON {
        return Biquad {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        };
    }
    b0 /= a0;
    b1 /= a0;
    b2 /= a0;
    a1 /= a0;
    a2 /= a0;
    Biquad { b0, b1, b2, a1, a2 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_shelf_and_high_shelf_match_rbj_cookbook_dc_and_nyquist_gain() {
        let fs = 48_000.0;
        let f0 = fs / 6.0; // 8 kHz: omega = pi/3, so alpha is large and 2*sqrt(A)*alpha != 2*sqrt(2*sqrt(A)*alpha)
        let gain_db = 12.0;
        let q = std::f64::consts::FRAC_1_SQRT_2;

        let omega = 2.0 * PI * f0 / fs;
        let sin = omega.sin();
        let cos = omega.cos();
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);
        let beta = 2.0 * a.sqrt() * alpha;
        let linear_gain = 10f64.powf(gain_db / 20.0);

        // LowShelf exact RBJ cookbook coefficients
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

        // LowShelf DC gain (z = 1) is linear_gain, Nyquist gain (z = -1) is 1.0
        let ls_dc = (ls.b0 + ls.b1 + ls.b2) / (1.0 + ls.a1 + ls.a2);
        let ls_nyquist = (ls.b0 - ls.b1 + ls.b2) / (1.0 - ls.a1 + ls.a2);
        assert!(
            (ls_dc - linear_gain).abs() < 1e-12,
            "ls_dc={ls_dc}, expected={linear_gain}"
        );
        assert!((ls_nyquist - 1.0).abs() < 1e-12, "ls_nyquist={ls_nyquist}");

        // HighShelf exact RBJ cookbook coefficients
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

        // HighShelf DC gain (z = 1) is 1.0, Nyquist gain (z = -1) is linear_gain
        let hs_dc = (hs.b0 + hs.b1 + hs.b2) / (1.0 + hs.a1 + hs.a2);
        let hs_nyquist = (hs.b0 - hs.b1 + hs.b2) / (1.0 - hs.a1 + hs.a2);
        assert!((hs_dc - 1.0).abs() < 1e-12, "hs_dc={hs_dc}");
        assert!(
            (hs_nyquist - linear_gain).abs() < 1e-12,
            "hs_nyquist={hs_nyquist}, expected={linear_gain}"
        );

        // Time-domain steady-state DC and Nyquist responses + denormal flushing
        let mut ls_state = BiquadState::default();
        let mut hs_state = BiquadState::default();
        let mut ls_out = 0.0;
        let mut hs_out = 0.0;
        for _ in 0..512 {
            ls_out = ls_state.process(&ls, 1.0);
            hs_out = hs_state.process(&hs, 1.0);
        }
        assert!((ls_out - linear_gain).abs() < 1e-9);
        assert!((hs_out - 1.0).abs() < 1e-9);

        let mut denorm_state = BiquadState::default();
        assert_eq!(denorm_state.process(&ls, 1e-20), 0.0);
        assert_eq!(denorm_state.y1, 0.0);
    }

    #[test]
    fn profile_path_rejects_traversal() {
        for bad in [
            "../etc/passwd",
            "a/b",
            "..",
            ".",
            "a\\b",
            "",
            "/etc/passwd",
            "./flat",
        ] {
            assert!(
                profile_path(bad).is_err(),
                "profile_path({bad:?}) must return Err"
            );
        }
    }

    #[test]
    fn parse_filter_rejects_trailing_unpaired_token() {
        for bad in [
            "Filter: ON PK Fc 1000 Hz Gain -3.0 dB Q",
            "Filter: ON PK Fc 1000 Hz trailing",
            "Filter: OFF LS Gain",
            "Filter: ON HS stray",
        ] {
            assert!(
                EqProfile::parse("bad", bad).is_err(),
                "expected trailing token error for: {bad}"
            );
        }
    }
}
