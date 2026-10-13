.pragma library

// Material You 3-like palette (dark) + layout ratios for Musika.
// Import in QML with: import "Theme.js" as Theme
//
// Everything is proportional: sizes are expressed as fractions of a container
// (usually the component's own width/height or the window), never as fixed px.

var radius = 3 // Global corner roundness.

// Motion: soft, cubic-bezier easing.
var duration = 100
var easeOut = [0.2, 0, 0, 1]

// Top-level layout ratios (fraction of the window).
var sidebarRatio = 0.15
var topBarRatio = 0.09
var playerRatio = 0.115

// 1px-ish hairlines, but proportional so they survive scaling.
var hairlineRatio = 0.0018

// Surfaces
var background = "#141218"
var onBackground = "#E6E0E9"
var surface = "#141218"
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
var error = "#F2B8B5"

// Lines
var outline = "#938F99"
var outlineVariant = "#49454F"
