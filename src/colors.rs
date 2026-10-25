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

//! Dynamic color engine (THM-1..6 / roadmap T0–T5): cover art → Material You
//! dark scheme.
//!
//! The pipeline is split so each stage can be cached and tuned on its own:
//!
//!   `seed`   — decode, downscale, quantize, score (dominant/colorful, DB6
//!              seed cache keyed by cover hash);
//!   `scheme` — seed → roles for the 2021 / 2025 / DMS palette specs;
//!   `engine` — options, DB6 `cover_color` rows, purge and the facade.
//!
//! matugen itself is GPL-2.0; only its engine lineage matters here. We embed
//! the MIT/Apache `material-colors` crate and reimplement the surrounding
//! pipeline, honoring DMS's seed/scheme split (`MATUGENSTUDY.md`).

#![allow(dead_code)] // some milestone APIs (previews/settings) land later.

macro_rules! name_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $text:literal $(| $alias:literal)*,
            )*
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )*
        }

        impl $name {
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)*
                }
            }

            pub fn from_name(value: &str) -> Option<Self> {
                match value.trim().to_ascii_lowercase().as_str() {
                    $($text $(| $alias)* => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

pub(crate) mod engine;
mod rgb;
pub(crate) mod scheme;
pub(crate) mod seed;

#[allow(unused_imports)]
pub use engine::{
    CoverScheme, Options, fetch, fetch_into, from_seed, generate_from_path, hash_roles, invalidate,
    invalidate_with, purge, purge_with,
};
#[allow(unused_imports)]
pub use scheme::{Spec, VariantChoice};
#[allow(unused_imports)]
pub use seed::SourceMode;
