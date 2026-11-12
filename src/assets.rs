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

//! Runtime asset paths and the shared icon vocabulary.
//!
//! The core daemon (tray), the GPUI child and the tests all resolve the same
//! `MUSIKA_ASSET_DIR`; the root and the multi-surface icon names live here once
//! instead of being restated by each child (tray, player bar, fullscreen).

use std::sync::OnceLock;

/// Transport glyphs shared by the player bar, the fullscreen strip and the
/// tray indicator, so those surfaces cannot drift apart.
pub(crate) const ICON_PLAY: &str = "play-1001-svgrepo-com.svg";
pub(crate) const ICON_PAUSE: &str = "pause-svgrepo-com.svg";
pub(crate) const ICON_PREVIOUS: &str = "skip-previous-solid-svgrepo-com.svg";
pub(crate) const ICON_NEXT: &str = "skip-next-circle-solid-svgrepo-com.svg";

/// The runtime asset root: `MUSIKA_ASSET_DIR` (the packaged share dir) or the
/// checkout's `src/` for a plain cargo build.
pub(crate) fn root() -> &'static str {
    static ROOT: OnceLock<String> = OnceLock::new();
    ROOT.get_or_init(|| root_from(std::env::var("MUSIKA_ASSET_DIR").ok().as_deref()))
}

/// Resolve the root from an optional env value. Empty or absent falls back to
/// the checkout path; pure so tests do not have to mutate the environment.
fn root_from(env: Option<&str>) -> String {
    env.filter(|path| !path.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/src", env!("CARGO_MANIFEST_DIR")))
}

/// Absolute path of one SVG asset under the runtime root.
pub(crate) fn icon(name: &str) -> String {
    format!("{}/icons/{name}", root())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_falls_back_to_the_checkout_and_honours_the_env() {
        assert_eq!(root_from(Some("/opt/musika/share")), "/opt/musika/share");
        assert!(root_from(None).ends_with("/src"));
        assert!(root_from(Some("")).ends_with("/src"));
        assert_eq!(
            icon(ICON_PAUSE),
            format!("{}/icons/{ICON_PAUSE}", root())
        );
    }
}
