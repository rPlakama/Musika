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

// UI-60: the star-count badge that rides a favourite mark — one
// `number-0..5.svg` digit in a small disc at the mark's bottom-right corner.
// 0 is a real value (the number-0 glyph); hosts decide whether a mark (and
// therefore the badge) is drawn at all.
Item {
    id: root

    /// 0..=5 (0 = unrated, drawn with the number-0 glyph).
    property int rating: 0
    property color symbolColor: Palette.primary
    property color backgroundColor: Palette.surface
    /// Glyph inset as a fraction of the box (smaller = bigger digit).
    property real glyphInset: 0.14

    implicitWidth: 22.4
    implicitHeight: 22.4

    // ARC-41: spelled out so the icon pipeline sees every glyph reference.
    readonly property var glyphs: [
        "qrc:/app/icons/number-0.svg",
        "qrc:/app/icons/number-1.svg",
        "qrc:/app/icons/number-2.svg",
        "qrc:/app/icons/number-3.svg",
        "qrc:/app/icons/number-4.svg",
        "qrc:/app/icons/number-5.svg"
    ]
    readonly property url glyph: root.glyphs[Math.max(0, Math.min(5, root.rating))]

    Rectangle {
        anchors.fill: parent
        radius: width * 0.5
        color: root.backgroundColor
        border.color: Palette.outlineVariant
        border.width: Math.max(1, height * 0.08)
    }

    Icon {
        anchors.fill: parent
        anchors.margins: parent.width * root.glyphInset
        source: root.glyph
        symbolColor: root.symbolColor
    }
}
