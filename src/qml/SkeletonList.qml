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

// UI-22: shared loading skeleton — four shrinking muted bars, the pattern the
// search columns shipped, extracted so every async surface can use the same
// wait state. Decorative only: no cursor, no input, no layout influence.
//
//     SkeletonList { anchors.fill: parent; visible: root.loading && count === 0 }
Item {
    id: root

    property int count: 4

    Column {
        anchors.fill: parent
        anchors.margins: root.height * 0.03
        spacing: root.height * 0.025

        Repeater {
            model: root.count

            delegate: Rectangle {
                required property int index

                width: parent.width * (1 - index * 0.12)
                height: root.height * 0.075
                radius: Theme.radius
                color: Palette.surfaceContainer
                opacity: 0.6
            }
        }
    }
}
