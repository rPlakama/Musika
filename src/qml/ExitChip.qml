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

// Exit/back chip: the tonal rectangle with the exit glyph and the key hint,
// shared by the album page, the artist page and the fullscreen player.
// `host` is the surface that owns the proportions (its height sizes the chip
// and its glyph, its width the padding):
//
//     ExitChip { host: root; onClicked: root.back() }
//
// `highlighted` marks the focus/selection tone (AlbumEntry's toolbar state);
// `padRatio` keeps each host's horizontal padding (default 0.04).
Rectangle {
    id: root

    required property Item host
    property string label: "ESC/Q"
    property bool highlighted: false
    property real padRatio: 0.04

    signal clicked()

    implicitWidth: content.implicitWidth + root.host.width * root.padRatio
    implicitHeight: root.host.height * 0.045
    radius: Theme.radius
    color: (root.highlighted || chipHover.hovered)
        ? Palette.secondaryContainer
        : Palette.surfaceContainerHigh

    Behavior on color { ColorAnim {} }

    // Exit glyph + key hint. SymbolIcon: white-baked asset, re-coloured to
    // the fixed strong glyph tone; the fixed frame keeps it still.
    Row {
        id: content
        anchors.centerIn: parent
        spacing: root.host.height * 0.01

        Item {
            anchors.verticalCenter: parent.verticalCenter
            width: root.host.height * 0.026
            height: width

            SymbolIcon {
                anchors.centerIn: parent
                source: "qrc:/app/icons/exit-2-svgrepo-com.svg"
                symbolColor: Theme.iconStrong
                width: parent.width * 0.85
                height: width
            }
        }

        FadeText {
            anchors.verticalCenter: parent.verticalCenter
            text: root.label
            color: Palette.onSurface
            font.pixelSize: root.host.height * 0.022
        }
    }

    HoverHandler {
        id: chipHover
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        onTapped: root.clicked()
    }
}
