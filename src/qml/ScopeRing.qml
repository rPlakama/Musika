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
import musika
import "Theme.js" as Theme

// Tab scope ring: one reusable hairline frame. It fills its parent (the
// stationary control) and marks it as the scoped target — the status bar's
// sync/trashcan and the artist entry's chips share it.
Rectangle {
    id: root

    property bool shown: false
    property real lineWidth: Math.max(1, root.height * Theme.hairlineRatio * 3.25)

    anchors.fill: parent
    radius: Theme.radius
    color: "transparent"
    border.width: root.lineWidth
    border.color: Palette.primary
    opacity: root.shown ? 1 : 0

    Behavior on opacity {
        NumberAnimation {
            duration: Theme.motion.micro
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.standard
        }
    }
}
