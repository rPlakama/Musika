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
// Shape (R3): one soft corner everywhere. Every Rectangle carries
// `Theme.radius` (the fullscreen queue/lyrics panes' corner); the only other
// shape is the disc (`shapeFull`) for artist avatars/art and icon badges.
// ---------------------------------------------------------------------------
var radius = 3 // Global soft corner roundness.

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
    scroll: 300,       // the viewport glide as the cursor steps through
                       // the centered band (scrolloff); rows drift up/down softly
    standard: 160,     // view push/pop/replace, overlay in/out
    emphasized: 240,   // fullscreen player, first paint of a context
    lazy: 320,         // toast out, nothing keyboard-critical
    spin: 1000,        // one revolution while a sync runs
    spinSlow: 3000     // one revolution on hover
}

// Global opt-out for decorative motion (FadeText, button presses). Flipped by
// a user setting later; functional motion (progress, seek) never honours it.
var reduceMotion = false

// LYR-8: a manual scroll pauses the lyrics centre-follow for this idle window
// before the active phrase re-centres; the wait shortens while the pane is
// not the focused/highlighted section.
var lyricsResume = 5000
var lyricsResumeUnfocused = 2000

var ease = {
    standard: [0.2, 0, 0, 1],
    decelerate: [0.05, 0.7, 0.1, 1],
    accelerate: [0.3, 0, 0.8, 0.15],
    // Overshooting release for press feedback (button scale-down bounce).
    bounce: [0.34, 1.56, 0.64, 1]
    // Note: Qt's Easing.Bezier clamps y control points above 1 (probed), so
    // CSS spring curves like cubic-bezier(.63,.12,.84,1.72) must be carried
    // by OutBack + easing.overshoot instead.
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
var topBarRatio = 0.0648
var playerRatio = 0.0828

// 1px-ish hairlines, but proportional so they survive scaling.
var hairlineRatio = 0.0018

// Responsive breakpoints (logical px, UI-32): one shared vocabulary instead of
// ad-hoc width checks. Clamps here are sanctioned like `fontSize`.
var breakpoints = { mini: 420, narrow: 640, compact: 900 }

function isNarrow(width) {
    return width < breakpoints.narrow
}

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

// ---------------------------------------------------------------------------
// UI-55: Acrylic panels (MS Fluent recipe; the R1/R2 depth amendment). A
// translucent tint over a blurred, desaturated copy of what sits behind the
// panel. `enabled: false` restores the solid role fill for accessibility and
// low-end hardware (the MS fallback list).
// ---------------------------------------------------------------------------
var acrylic = {
    enabled: true,
    tintAlpha: 1.0,   // tint opacity over the blur (ARC-IDK1: 40% less
                      // transparent than 0.76, clamped to opaque)
    blur: 0.8,        // MultiEffect blur amount
    blurMax: 64,      // blur kernel ceiling
    saturation: 0.6,  // niri-like desaturation (1.6x)
    downscale: 0.5,   // backdrop texture fraction (the blur hides it)
    noise: 0.03       // MS grain over the tint
}

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

// Favourite star, lit: Material Yellow 400. A semantic state colour like the
// error role, so it stays fixed across dynamic schemes (ARC-42).
var favoriteStar = "#FFD54F"

// ---------------------------------------------------------------------------
// Lyrics stage (see roadmap "Lyrics presentation"): a focus window where the
// active line is scaled and fully contrasted while neighbours dim with
// distance. Unsynced lyrics use one uniform `contentDim` column.
// ---------------------------------------------------------------------------
var contentDim = 0.8      // near lines / whole unsynced column
var contentDimFar = 0.5   // far lines in the synced focus window
var lyricsEmphasis = 1.12 // active-line scale (transform, never font size)
// UI-45 selection stage: the cursor/hover row's primary text leads its
// neighbours (transform only, so rows never reflow). The centred scroll
// (scrolloff) keeps the cursor clear of the pane edges, so the stage can
// read larger: 20% over the row.
var selectionEmphasis = 1.20

// ---------------------------------------------------------------------------
// Font families. One app face: Overused Grotesk Roman stands in for the
// unpackaged Bricolage Grotesque and covers wordmark, lyrics and body alike;
// roles stay so adding a second face later is a one-line change.
// ---------------------------------------------------------------------------
var fonts = {
    body: "Overused Grotesk Roman",
    lyrics: "Overused Grotesk Roman",
    wordmark: "Overused Grotesk Roman"
}

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

// UI-57: one width-driven type ramp for the chrome (list header title, status
// bar, player bar) — 1 at `compact` and above, tapering linearly to `floor`
// (default 0.7) at the `narrow` breakpoint, so type shrinks with the window
// instead of only with its height. Continuous, so resizing or crossing a
// breakpoint never jumps.
function widthScale(width, floor) {
    var base = (floor === undefined) ? 0.7 : floor
    return clamp(base + (1 - base) * ((width - breakpoints.narrow)
        / (breakpoints.compact - breakpoints.narrow)), base, 1)
}

// UI-58: adaptive rows. A row may lose at most 18% of its baseline
// (default-window) height; past that the squeeze trades whole rows — the
// count drops by one and the rest land back in [0.82, 1]× the baseline —
// instead of shrinking every row further. ARC-UI-3: the edge sits at 0.82,
// so the reduction initiates 40% sooner than the original 0.7 band (user
// call). `ratio` is the host's nominal row fraction of its viewport;
// `baseline` the row height captured on the first real measure (0 before
// that: the nominal is returned untouched).
function adaptiveRow(viewHeight, ratio, baseline) {
    var nominal = viewHeight * ratio
    if (baseline <= 0 || nominal <= 0)
        return nominal
    var edge = 0.82
    if (nominal >= edge * baseline)
        return nominal
    var rows = Math.max(1, Math.floor(viewHeight / (edge * baseline)))
    return viewHeight / rows
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

// VI-11: `gg`/`G` land a list cursor on the first/last row and keep it
// visible. `last` picks the edge; `mode` is the ListView position mode
// (callers pass ListView.Contain — a `.pragma library` has no QtQuick globals).
// Empty lists are no-ops. The view is snapped even when the cursor already
// sits on the edge: the view can be elsewhere (mouse scroll, a lagging
// follow), and skipping the snap made the next key animatively swoosh the
// whole way back.
function cursorEdge(list, last, mode) {
    if (!list || list.count === 0)
        return;
    var target = last ? list.count - 1 : 0;
    list.currentIndex = target;
    list.positionViewAtIndex(target, mode);
}

// UI-45: cap a selection emphasis so the scaled text stops at its own box
// edge (the lyrics stage does the same against the pane margin). `room` is
// the text box width, `needed` its painted width.
function fitScale(room, needed, max) {
    if (room <= 0 || needed <= 0)
        return max;
    return Math.min(max, room / needed);
}

// Shared "m:ss" (h:mm:ss past the hour), used by every time label (UI-26).
function formatTime(seconds) {
    var total = Math.max(0, Math.floor(seconds || 0))
    var h = Math.floor(total / 3600)
    var m = Math.floor((total % 3600) / 60)
    var s = total % 60
    var mm = (h > 0 && m < 10 ? "0" : "") + m
    return (h > 0 ? h + ":" + mm : String(m)) + ":" + (s < 10 ? "0" : "") + s
}
