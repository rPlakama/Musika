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

import QtQuick
import "Theme.js" as Theme

// Text that fades in when it is created. Drop-in replacement for `Text`.
// Set `animated: false` for content that is re-created while scrolling
// (list/search delegates) so it does not blink.
Text {
    id: root

    property bool animated: true

    font.family: Theme.fonts.body

    NumberAnimation on opacity {
        running: root.animated && !Theme.reduceMotion
        from: 0
        to: 1
        duration: Theme.motion.quick
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
    }
}
