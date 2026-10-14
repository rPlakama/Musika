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

// UI-54: session scroll memory. Pages save their cursor and viewport offset
// here continuously (while alive), so the values are current the moment the
// page is destroyed, and restore them on the first payload after reopening.
var scrollPositions = ({})

function saveScroll(key, index, y) {
    if (!key)
        return
    scrollPositions[key] = { index: index, y: y }
}

function loadScroll(key) {
    if (!key || scrollPositions[key] === undefined)
        return null
    return scrollPositions[key]
}
