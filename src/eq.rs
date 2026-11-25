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
//! Parsing is deliberately strict: a profile either compiles cleanly into an
//! MPV `lavfi` filter chain or is rejected with the line number, never
//! silently skipped.

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

    /// Compiles this profile into an MPV `af` property value using FFmpeg's
    /// `lavfi` audio filters (`lavfi=[...]`). Returns an empty string when the
    /// profile is a no-op (zero preamp and no active filters).
    pub fn to_mpv_af(&self) -> String {
        let mut stages = Vec::with_capacity(self.filters.len() + 1);
        if self.preamp_db.abs() >= 1e-4 {
            stages.push(format!("volume={:.3}dB", self.preamp_db));
        }
        for filter in &self.filters {
            let stage = match filter.kind {
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
            };
            stages.push(stage);
        }
        if stages.is_empty() {
            String::new()
        } else {
            format!("lavfi=[{}]", stages.join(","))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_mpv_af_compiles_preamp_and_all_filter_kinds() {
        let profile = EqProfile::parse(
            "Full",
            "Preamp: -4.5 dB\n\
             Filter: ON PK Fc 1000 Hz Gain 3.0 dB Q 1.41\n\
             Filter: ON LS Fc 80 Hz Gain 4.0 dB Q 0.707\n\
             Filter: ON HS Fc 8000 Hz Gain -2.0 dB Q 0.707\n\
             Filter: ON LP Fc 16000 Hz Q 0.707\n\
             Filter: ON HP Fc 30 Hz Q 0.707\n\
             Filter: ON BP Fc 500 Hz Q 2.0\n\
             Filter: ON NO Fc 60 Hz Q 8.0\n",
        )
        .expect("parse full eq");
        let af = profile.to_mpv_af();
        assert!(af.starts_with("lavfi=[") && af.ends_with(']'), "af={af}");
        assert!(af.contains("volume=-4.500dB"), "af={af}");
        assert!(af.contains("equalizer=f=1000.000:width_type=q:w=1.4100:g=3.000"));
        assert!(af.contains("lowshelf=f=80.000:width_type=q:w=0.7070:g=4.000"));
        assert!(af.contains("highshelf=f=8000.000:width_type=q:w=0.7070:g=-2.000"));
        assert!(af.contains("lowpass=f=16000.000:width_type=q:w=0.7070"));
        assert!(af.contains("highpass=f=30.000:width_type=q:w=0.7070"));
        assert!(af.contains("bandpass=f=500.000:width_type=q:w=2.0000"));
        assert!(af.contains("bandreject=f=60.000:width_type=q:w=8.0000"));

        let flat = EqProfile::parse("Flat", "Preamp: 0 dB\nFilter: OFF PK Fc 1000 Hz Gain 3 dB\n")
            .expect("parse flat eq");
        assert!(flat.to_mpv_af().is_empty());
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
