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

//! Backend-agnostic Equalizer APO profile parser and loader.
//!
//! Profiles live in `$XDG_CONFIG_HOME/musika/eqs/*.{txt,eq}` and support the
//! Equalizer APO / AutoEq / Peace text subset:
//! - `Preamp: <dB> dB`
//! - `Filter:` and numbered `Filter <N>:` (`ON|OFF`, `PK|PEQ|LS|LSC|LSQ|HS|HSC|HSQ|LP|LPQ|HP|HPQ|BP|NO|NOTCH`,
//!   `Fc <Hz>`, `Gain <dB>`, `Q <q>` or `BW Oct <oct>`)
//! - `GraphicEQ: <freq> <gain>; ...` (up to 128 piecewise-linear bands)
//! - `Device:`, `Channel:`, and `Stage:` scoping directives (accepted and ignored).
//!
//! Translation to MPV's `lavfi` audio filter syntax lives in
//! [`crate::player::mpv::eq`], invoked from the daemon/IPC layer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const MAX_FILTERS: usize = 64;
const MAX_GRAPHIC_BANDS: usize = 128;
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

/// One piecewise-linear frequency/gain control point from a `GraphicEQ:` line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphicBand {
    pub freq: f64,
    pub gain_db: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EqProfile {
    pub name: String,
    pub preamp_db: f64,
    pub filters: Vec<EqFilter>,
    pub graphic_bands: Vec<GraphicBand>,
}

impl EqProfile {
    /// Parses one Equalizer APO–style profile. `name` is the display name
    /// (normally the file stem).
    pub fn parse(name: &str, text: &str) -> Result<Self> {
        let mut preamp_db = 0.0f64;
        let mut filters = Vec::new();
        let mut graphic_bands = Vec::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (directive, rest) = line
                .split_once(':')
                .with_context(|| format!("line {}: expected 'Directive: value'", line_no + 1))?;
            let directive = directive.trim().to_ascii_lowercase();
            match directive.as_str() {
                "preamp" => {
                    let value = parse_first_number(rest)
                        .with_context(|| format!("line {}: preamp", line_no + 1))?;
                    if !value.is_finite() {
                        bail!("line {}: preamp must be finite", line_no + 1);
                    }
                    preamp_db = value.clamp(-60.0, MAX_GAIN_DB);
                }
                dir if is_filter_directive(dir) => {
                    if let Some(filter) =
                        parse_filter(rest).with_context(|| format!("line {}", line_no + 1))?
                    {
                        filters.push(filter);
                    }
                }
                "graphiceq" => {
                    let bands =
                        parse_graphic(rest).with_context(|| format!("line {}", line_no + 1))?;
                    graphic_bands.extend(bands);
                }
                // Equalizer APO may scope a file to a device/channel/stage;
                // Musika applies the profile to the active stream.
                "device" | "channel" | "stage" => {}
                other => bail!("line {}: unsupported directive '{other}'", line_no + 1),
            }
            if filters.len() > MAX_FILTERS {
                bail!("too many filters (max {MAX_FILTERS})");
            }
            if graphic_bands.len() > MAX_GRAPHIC_BANDS {
                bail!("too many GraphicEQ bands (max {MAX_GRAPHIC_BANDS})");
            }
        }
        graphic_bands.sort_by(|a, b| a.freq.total_cmp(&b.freq));
        Ok(Self {
            name: name.to_string(),
            preamp_db,
            filters,
            graphic_bands,
        })
    }
}

/// Accepts `Filter:` and numbered `Filter 1:`, `Filter 2:`, ... (AutoEq / Peace / REW).
fn is_filter_directive(lower: &str) -> bool {
    if lower == "filter" {
        return true;
    }
    let Some(rest) = lower.strip_prefix("filter") else {
        return false;
    };
    let rest = rest.trim();
    !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit())
}

/// Converts bandwidth in octaves (`BW Oct`) to quality factor `Q`.
fn octaves_to_q(octaves: f64) -> Result<f64> {
    if !octaves.is_finite() || !(0.01..=10.0).contains(&octaves) {
        bail!("BW Oct {octaves} is outside 0.01..=10.0");
    }
    let two_bw = 2f64.powf(octaves);
    let q = two_bw.sqrt() / (two_bw - 1.0);
    Ok(q.clamp(MIN_Q, MAX_Q))
}

/// `ON PK Fc 1000 Hz Gain -3 dB Q 1.4` or `BW Oct 0.5`; an `OFF` line is a no-op.
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
        "LS" | "LSC" | "LSQ" => FilterKind::LowShelf,
        "HS" | "HSC" | "HSQ" => FilterKind::HighShelf,
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
            "bw" => {
                if !tokens[index + 1].eq_ignore_ascii_case("oct") {
                    bail!("expected 'BW Oct <value>', got 'BW {}'", tokens[index + 1]);
                }
                let Some(val_token) = tokens.get(index + 2) else {
                    bail!("missing value after 'BW Oct'");
                };
                let octaves: f64 = val_token
                    .parse()
                    .with_context(|| format!("invalid BW Oct '{val_token}'"))?;
                q = octaves_to_q(octaves)?;
                index += 3;
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
    if !freq.is_finite() || !(1.0..=48_000.0).contains(&freq) {
        bail!("Fc {freq} Hz is outside 1..=48000");
    }
    if !gain_db.is_finite() || !(-MAX_GAIN_DB..=MAX_GAIN_DB).contains(&gain_db) {
        bail!("Gain {gain_db} dB is outside ±{MAX_GAIN_DB}");
    }
    if !q.is_finite() {
        bail!("Q must be finite");
    }
    Ok(Some(EqFilter {
        kind,
        freq,
        gain_db: gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
        q: q.clamp(MIN_Q, MAX_Q),
    }))
}

/// `20 -3; 25 -3; ...` piecewise-linear frequency/gain control points.
fn parse_graphic(rest: &str) -> Result<Vec<GraphicBand>> {
    let mut bands = Vec::new();
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
        if parts.next().is_some() {
            bail!("unexpected extra token in GraphicEQ band '{pair}'");
        }
        if !freq.is_finite() || !(1.0..=48_000.0).contains(&freq) {
            bail!("GraphicEQ frequency {freq} Hz is outside 1..=48000");
        }
        if !gain.is_finite() {
            bail!("GraphicEQ gain must be finite");
        }
        bands.push(GraphicBand {
            freq,
            gain_db: gain.clamp(-MAX_GAIN_DB, MAX_GAIN_DB),
        });
        if bands.len() > MAX_GRAPHIC_BANDS {
            bail!("too many GraphicEQ bands (max {MAX_GRAPHIC_BANDS})");
        }
    }
    Ok(bands)
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
    fn parses_autoeq_numbered_filters_shelves_and_bw_oct() {
        let profile = EqProfile::parse(
            "AutoEq",
            "Preamp: -5.4 dB\n\
             Stage: pre-mix\n\
             Filter 1: ON LSC Fc 105 Hz Gain 4.5 dB Q 0.71\n\
             Filter 2: ON PK Fc 1000 Hz Gain -2.0 dB BW Oct 1.0\n\
             Filter 3: ON HSQ Fc 10000 Hz Gain 1.5 dB Q 0.70\n",
        )
        .expect("parse autoeq");
        assert_eq!(profile.filters.len(), 3);
        assert_eq!(profile.filters[0].kind, FilterKind::LowShelf);
        assert_eq!(profile.filters[1].kind, FilterKind::Peak);
        assert!((profile.filters[1].q - std::f64::consts::SQRT_2).abs() < 1e-6);
        assert_eq!(profile.filters[2].kind, FilterKind::HighShelf);
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
            "Filter: ON PK Fc 1000 Hz BW Oct",
        ] {
            assert!(
                EqProfile::parse("bad", bad).is_err(),
                "expected trailing token error for: {bad}"
            );
        }
    }
}
