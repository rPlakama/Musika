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

// UI-58 guard: the adaptive row must be continuous. The old row-count step
// jumped ~7 px for a 1 px viewport change; this sweep fails on anything close.
import QtQuick
import "../src/qml/Theme.js" as Theme

Item {
    Component.onCompleted: {
        var ratio = 0.105
        var base = 774 * ratio
        var floor = 0.82 * base
        var prev = Theme.adaptiveRow(774, ratio, base)
        var worst = 0
        var code = 0
        for (var i = 1; i <= 4000 && code === 0; ++i) {
            var row = Theme.adaptiveRow(774 - i * 0.1, ratio, base)
            if (row < floor - 1e-6) {
                code = 2
                break
            }
            var delta = Math.abs(row - prev)
            if (delta > worst)
                worst = delta
            if (delta > 0.2)
                code = 3
            prev = row
        }
        if (code === 0 && worst >= 0.2)
            code = 4
        if (code === 0
                && Math.abs(Theme.adaptiveRow(500, ratio, base) - floor) > 1e-6)
            code = 5
        if (code === 0 && Theme.adaptiveRow(774, 0, base) !== 0)
            code = 6
        Qt.exit(code)
    }
}
