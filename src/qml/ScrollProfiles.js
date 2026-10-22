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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

.pragma library

// SCROLLMODAL: one context profile carries the scaling policy (metrics) and
// the context-only keyboard permissions (flags). ScrollContent/ScrollPanel
// read these as property defaults; a host assignment still overrides them.

var profiles = {
    library: {
        headerRatio: 0.14,
        rowRatio: 0.105,
        columns: 4,
        cellAspect: 0.78,
        cellExtra: 0.085,
        bleedRatio: 0.012
    },
    album: {
        rowRatio: 0.055,
        bleedRatio: 0.012
    },
    search: {
        rowRatio: 0.12,
        rowHeightScale: 1.04,
        bleedRatio: 0.012
    },
    queue: {
        rowRatio: 0.10,
        bleedRatio: 0.012,
        flags: { reorder: true, remove: true }
    },
    lyrics: {
        rowRatio: 0
    }
}

function get(name) {
    return profiles[name] || {}
}

/// Whole rows that fit in `avail`, leaving the remainder as surface.
function snapBody(avail, stride, gap) {
    if (stride <= 0)
        return avail
    var spacing = gap === undefined ? 0 : gap
    var rows = Math.max(1, Math.floor((avail + spacing) / stride + 1e-6))
    return Math.min(avail, rows * stride - spacing)
}
