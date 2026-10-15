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

// VI-4: the "N selected" pill a surface floats while visual selection is
// active. Colours match the selected rows (secondaryContainer) so the pill
// and the highlighted range read as one state.
//
// The width only ever grows while a selection lives: an invisible frame
// (the anchor) reserves the pill's width, and the visible background
// extends into it when the count's text grows. Navigating j/k through the
// range therefore never reflows or shakes the pill, and the count's right
// edge stays put.
Item {
    id: root

    property int count: 0
    /// Narrow headers (search columns) get the short "N sel" copy.
    property bool compact: false
    /// Host-set text size; must not derive from this item's own height or the
    /// implicit sizes would loop (font -> text -> implicitHeight -> font).
    property real pixelSize: 12

    visible: count > 0
    implicitWidth: frame.width
    implicitHeight: Math.max(pixelSize * 1.8, label.implicitHeight * 1.3)

    // Ratchet: the widest text seen for the current selection; reset when the
    // selection clears so the next one starts snug again.
    property real _textWidth: 0
    onCountChanged: if (count === 0) _textWidth = 0;

    // The invisible anchor the pill grows into; only its width changes.
    Rectangle {
        id: frame
        visible: false
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: Math.max(root.pixelSize * 1.8, root._textWidth + root.pixelSize * 1.4)
        height: parent.height
    }

    // The visible background, anchored to the frame; it extends only when
    // the frame does.
    Rectangle {
        id: pill
        anchors.right: frame.right
        anchors.verticalCenter: parent.verticalCenter
        width: frame.width
        height: frame.height
        radius: Theme.radius
        color: Palette.secondaryContainer
        border.color: Palette.primary
        border.width: Math.max(1, root.pixelSize * 0.08)
    }

    // Right-aligned: the number's right edge never moves; the ratchet gives
    // the pill room only when the text actually grows.
    FadeText {
        id: label
        anchors.right: pill.right
        anchors.rightMargin: root.pixelSize * 0.7
        anchors.verticalCenter: pill.verticalCenter
        text: root.compact ? root.count + " sel" : root.count + " selected"
        color: Palette.onSecondaryContainer
        font.pixelSize: root.pixelSize
        animated: false
        onImplicitWidthChanged: {
            if (root.count > 0)
                root._textWidth = Math.max(root._textWidth, implicitWidth);
        }
    }
}
