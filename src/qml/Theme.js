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

.pragma library

// Material You 3-like palette (dark) + tokens for Musika.
// Import in QML with: import "Theme.js" as Theme
//
// Everything is proportional: sizes are expressed as fractions of a container
// (usually the component's own width/height or the window), never as fixed px.
// The only sanctioned pixel values are clamps (fontSize) and hairlines.
//
// See materialyou3idea.md for the rationale behind the token sets.

// ---------------------------------------------------------------------------
// Shape (R3): binary + one. Square is the default; soft only for interactive
// surfaces; full-circle only for artist avatars.
// ---------------------------------------------------------------------------
var radius = 3 // Global soft corner roundness.
var shapeSquare = 0

function shapeFull(size) {
    return size * 0.5
}

// ---------------------------------------------------------------------------
// Motion (R4/R5): tiers, not one global duration. If the next keystroke can
// race the animation, use instant/micro.
// ---------------------------------------------------------------------------
var motion = {
    instant: 0,        // mode switches, cursor moves, showcmd
    flash: 30,         // fullscreen enter/leave highlight fade
    micro: 80,         // hover/press layers, region focus border
    quick: 120,        // toasts in, row cursor, column switch
    standard: 160,     // view push/pop/replace, overlay in/out
    emphasized: 240,   // fullscreen player, first paint of a context
    lazy: 320,         // toast out, nothing keyboard-critical
    spin: 1000,        // one revolution while a sync runs
    spinSlow: 3000     // one revolution on hover
}

// Global opt-out for decorative motion (FadeText, blooms). Flipped by a user
// setting later; functional motion (progress, seek) never honours it.
var reduceMotion = false

var ease = {
    standard: [0.2, 0, 0, 1],
    decelerate: [0.05, 0.7, 0.1, 1],
    accelerate: [0.3, 0, 0.8, 0.15]
}

// Legacy aliases; new code uses motion.* / ease.*.
var duration = motion.quick
var easeOut = ease.standard

// ---------------------------------------------------------------------------
// Layering (z) — a single ladder so future surfaces don't invent values.
// ---------------------------------------------------------------------------
var zRegion = 10
var zToast = 20
var zOverlay = 30
var zCommand = 40
var zWhichKey = 50
var zTooltip = 60

// Top-level layout ratios (fraction of the window).
var sidebarRatio = 0.15
var topBarRatio = 0.09
var playerRatio = 0.115

// 1px-ish hairlines, but proportional so they survive scaling.
var hairlineRatio = 0.0018

// ---------------------------------------------------------------------------
// Surfaces (R1): hierarchy is tonal, never shadows.
// ---------------------------------------------------------------------------
var background = "#141218"
var onBackground = "#E6E0E9"
var surface = "#141218"
var surfaceDim = "#141218"
var surfaceBright = "#3B383E"
var surfaceContainerLowest = "#0F0D13"
var surfaceContainerLow = "#1D1B20"
var surfaceContainer = "#211F26"
var surfaceContainerHigh = "#2B2930"
var surfaceContainerHighest = "#36343B"
var onSurface = "#E6E0E9"
var surfaceVariant = "#49454F"
var onSurfaceVariant = "#CAC4D0"

// Accents
var primary = "#D0BCFF"
var onPrimary = "#381E72"
var primaryContainer = "#4F378B"
var onPrimaryContainer = "#EADDFF"
var secondary = "#CCC2DC"
var secondaryContainer = "#4A4458"
var onSecondaryContainer = "#E8DEF8"
var tertiary = "#EFB8C8"
var tertiaryContainer = "#633B48"
var onTertiaryContainer = "#FFD8E4"
var error = "#F2B8B5"
var errorContainer = "#8C1D18"
var onErrorContainer = "#F9DEDC"

// Inverse + tint (reserved; no shadows).
var inverseSurface = "#E6E0E9"
var onInverseSurface = "#322F35"
var surfaceTint = "#D0BCFF"

// Lines
var outline = "#938F99"
var outlineVariant = "#49454F"

// ---------------------------------------------------------------------------
// Modal scrim (R2): the one place blur/dim is allowed.
// ---------------------------------------------------------------------------
var scrim = "#000000"
var scrimOpacity = 0.5

// Fixed glyphs: play/pause stays white on every surface and every seeded
// palette (UI-31); it is the one glyph that never follows the theme roles.
var iconStrong = "#FFFFFF"

// ---------------------------------------------------------------------------
// State layers (R6): overlay colors on the base surface, pre-mixed AARRGGBB.
// ---------------------------------------------------------------------------
var stateHover = "#14E6E0E9"            // 8% onSurface
var statePress = "#1AE6E0E9"            // 10% onSurface
var stateDrag = "#29E6E0E9"             // 16% onSurface
var stateFocus = "#1AD0BCFF"            // 10% primary
var stateDisabledContainer = "#1F36343B" // 12% surfaceContainerHighest
var stateDisabledContent = "#61E6E0E9"   // 38% onSurface

// ---------------------------------------------------------------------------
// Lyrics stage (see roadmap "Lyrics presentation"): a focus window where the
// active line is scaled and fully contrasted while neighbours dim with
// distance. Unsynced lyrics use one uniform `contentDim` column.
// ---------------------------------------------------------------------------
var contentDim = 0.8      // near lines / whole unsynced column
var contentDimFar = 0.5   // far lines in the synced focus window
var lyricsEmphasis = 1.12 // active-line scale (transform, never font size)

// ---------------------------------------------------------------------------
// Typography roles (R7): proportional fractions with pixel clamps so labels
// never get tiny on large displays or dominate small windows.
// ---------------------------------------------------------------------------
var type = {
    display: [0.055, 22, 44],  // NowPlaying title
    headline: [0.04, 18, 32],  // album/section titles
    title: [0.032, 14, 24],    // region headers, column headers
    body: [0.024, 12, 18],     // list rows, subtitles
    label: [0.02, 10, 14],     // hints, keys, meta, counts
    mono: [0.02, 10, 14]       // timecodes, counts, showcmd
}

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value))
}

function fontSize(role, reference) {
    var spec = type[role]
    if (!spec)
        return reference * 0.02
    return clamp(reference * spec[0], spec[1], spec[2])
}

// Speaker icon by volume tier: muted / silent / low (≤33) / high. Shared by
// the player bar and the fullscreen transport (Q-4/P-7).
function volumeIcon(volume, muted) {
    if (muted)
        return "qrc:/app/icons/vol-mute-svgrepo-com.svg"
    if (volume <= 0)
        return "qrc:/app/icons/vol-zero-svgrepo-com.svg"
    if (volume <= 33)
        return "qrc:/app/icons/vol-low-svgrepo-com.svg"
    return "qrc:/app/icons/vol-high-svgrepo-com.svg"
}
