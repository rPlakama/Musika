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

// UI-52: the one reusable action chip — the artist header's download /
// shuffle / favourite cluster and the fullscreen keybar's album actions.
// `unit` is the chip's square side; the host sizes it from its own geometry
// (the artist header's page height, the keybar's bar height). Download
// visuals are the shared ARC-UI-6 DownloadButton, placed by the host.
Rectangle {
    id: chip

    property real unit: 32
    property string label: ""
    property url icon: ""
    property color iconColor: Palette.onSurfaceVariant

    signal clicked()

    // Icon-only chips are square; a label adds the text + side padding.
    // The implicit size mirrors the real one so chips survive inside
    // Layouts (a bare `width` reads as 0 there and collapses them).
    implicitWidth: chip.label.length > 0 ? content.implicitWidth + chip.unit * 0.8 : chip.unit
    implicitHeight: chip.unit
    width: implicitWidth
    height: implicitHeight
    radius: Theme.radius
    color: chipHover.hovered ? Palette.secondaryContainer : Palette.surfaceContainerHigh
    border.color: Palette.outlineVariant
    border.width: Theme.hairline(chip.unit)

    Behavior on color { ColorAnim {} }

    Row {
        id: content
        anchors.centerIn: parent
        spacing: chip.height * 0.18

        SymbolIcon {
            anchors.verticalCenter: parent.verticalCenter
            visible: chip.icon.toString().length > 0
            source: chip.icon
            symbolColor: chip.iconColor
            width: chip.height * 0.42
            height: width
        }

        FadeText {
            id: chipLabel
            anchors.verticalCenter: parent.verticalCenter
            text: chip.label
            color: Palette.onSurface
            font.pixelSize: chip.unit * 0.267
            animated: false
        }
    }

    HoverHandler {
        id: chipHover
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        onTapped: chip.clicked()
    }
}
