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
import QtQuick.Controls
import musika
import "Theme.js" as Theme

// Native Slider with Musika's flat rail: the whole control is the hit area,
// the visible rail is `trackHeight` and has no handle. Values are 0..1; the
// host binds `value` and applies the change from `moved`.
Slider {
    id: root

    property real trackHeight: 2
    property color trackColor: Palette.outlineVariant
    property color fillColor: Palette.primary
    property color mutedFillColor: Palette.outline
    property bool dimmed: false

    from: 0
    to: 1
    stepSize: 0
    live: true
    padding: 0

    background: Item {
        anchors.fill: parent

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width
            height: root.trackHeight
            radius: Theme.radius
            color: root.trackColor

            Rectangle {
                width: parent.width * root.visualPosition
                height: parent.height
                radius: Theme.radius
                color: root.dimmed ? root.mutedFillColor : root.fillColor
            }
        }
    }

    handle: Item {
        implicitWidth: 0
        implicitHeight: 0
    }

    HoverHandler {
        cursorShape: Qt.PointingHandCursor
    }
}
