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
import QtQuick.Effects
import QtQuick.VectorImage
import musika
import "Theme.js" as Theme

// Reusable recolourable SVG icon (roadmap D-6). Assets keep baked white fills
// (VectorImage cannot tint); `symbolColor` re-colours them through
// MultiEffect colorization, so any QML module can request a palette role
// without touching the asset. The caller owns the fixed frame; the glyph is
// the largest centred square inside it.
Item {
    id: root

    property url source
    property color symbolColor: Palette.onSurface
    /// Glyph size as a fraction of the smaller frame side (1.0 = full bleed).
    property real iconRatio: 1.0

    implicitWidth: 0
    implicitHeight: 0

    VectorImage {
        id: glyph
        anchors.centerIn: parent
        source: root.source
        preferredRendererType: VectorImage.CurveRenderer
        width: Math.min(root.width, root.height) * root.iconRatio
        height: width
        visible: false
    }

    MultiEffect {
        anchors.fill: glyph
        source: glyph
        colorization: 1.0
        colorizationColor: root.symbolColor
        visible: root.source.toString().length > 0
    }
}
