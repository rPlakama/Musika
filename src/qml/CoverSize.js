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

// Cover decode buckets (long edge, physical px). A large surface asks the
// backend for the smallest bucket that covers its painted size, capped by the
// screen's physical edge. Buckets keep one file per size class instead of a
// new `_NxN` file for every resize, and keep the backend from opening the
// provider's full-resolution art for a 700 px pane.
var buckets = [384, 512, 640, 768, 960, 1280, 1536, 1920, 2560, 3072, 4096]

/// Target long edge for `logicalSize` at `devicePixelRatio`, capped by
/// `screenMax` (logical screen edge) × DPR. 0 means "no resized file": the
/// surface is small enough for the shared 320 px thumbnail / original path.
function target(logicalSize, devicePixelRatio, screenMax) {
    var size = Number(logicalSize)
    if (!(size > 0))
        return 0
    var dpr = Number(devicePixelRatio)
    if (!(dpr > 0))
        dpr = 1
    var px = Math.ceil(size * dpr)
    var cap = Number(screenMax)
    if (cap > 0)
        px = Math.min(px, Math.ceil(cap * dpr))
    // Keep in sync with `THUMB_EDGE` in src/covers.rs: at or below the thumb
    // edge the existing thumbnail is already the right file.
    if (px <= 320)
        return 0
    for (var i = 0; i < buckets.length; ++i) {
        if (px <= buckets[i])
            return buckets[i]
    }
    return buckets[buckets.length - 1]
}
