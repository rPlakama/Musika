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
Rectangle {
    id: root

    property int count: 0
    /// Narrow headers (search columns) get the short "N sel" copy.
    property bool compact: false

    visible: count > 0
    implicitWidth: label.implicitWidth + height * 0.7
    implicitHeight: Math.max(16, label.implicitHeight * 1.9)
    radius: Theme.radius
    color: Palette.secondaryContainer
    border.color: Palette.primary
    border.width: Math.max(1, height * Theme.hairlineRatio)

    FadeText {
        id: label
        anchors.centerIn: parent
        text: root.compact ? root.count + " sel" : root.count + " selected"
        color: Palette.onSecondaryContainer
        font.pixelSize: Math.max(10, root.height * 0.5)
        animated: false
    }
}
