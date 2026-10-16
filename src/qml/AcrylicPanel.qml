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
import musika
import "Theme.js" as Theme

// UI-55: MS Fluent acrylic panel — a translucent tint over a blurred,
// desaturated copy of what sits behind the panel, plus the panel's own
// border/radius. Drop-in for a background Rectangle: callers keep their
// anchors, size, border and the children drawn above it, and only point
// `backdrop` at the item the panel overlays. `Theme.acrylic.enabled: false`
// (or no backdrop) paints the plain role fill — the MS fallback to solid.
Rectangle {
    radius: Theme.radius
    id: root

    /// The item behind the panel whose pixels feed the acrylic pass.
    property Item backdrop: null
    /// The opaque fallback/tint role (today's fill); acrylic draws it at
    /// `tintAlpha` over the blur.
    property color tint: Palette.surfaceContainerHigh
    property real tintAlpha: Theme.acrylic.tintAlpha

    readonly property bool acrylic: Theme.acrylic.enabled && root.backdrop !== null

    color: root.acrylic ? "transparent" : root.tint

    // Backdrop pass: the panel rect mapped into the backdrop, rendered at a
    // fraction of the panel size (the blur hides the downscale) and run
    // through the shared MultiEffect recipe (CoverImage/Home pass).
    ShaderEffectSource {
        id: backdropTexture
        anchors.fill: parent
        visible: root.acrylic
        sourceItem: root.backdrop
        live: true
        textureSize: Qt.size(
            Math.max(1, Math.ceil(root.width * Theme.acrylic.downscale)),
            Math.max(1, Math.ceil(root.height * Theme.acrylic.downscale)))
        sourceRect: {
            if (!root.backdrop)
                return Qt.rect(0, 0, 0, 0);
            var p = root.mapToItem(root.backdrop, 0, 0);
            return Qt.rect(p.x, p.y, root.width, root.height);
        }

        layer.enabled: root.acrylic
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: Theme.acrylic.blur
            blurMax: Theme.acrylic.blurMax
            saturation: Theme.acrylic.saturation
            autoPaddingEnabled: false
        }
    }

    // Translucent tint over the blur; the no-backdrop fallback stays opaque
    // on the root's own colour above.
    Rectangle {
        anchors.fill: parent
        visible: root.acrylic
        radius: root.radius
        color: Qt.rgba(root.tint.r, root.tint.g, root.tint.b, root.tintAlpha)
    }

    // MS grain: a tiled noise sheet at `Theme.acrylic.noise` (~0.03) over the
    // tint, so the material never reads as flat glass. Qt 6 no longer takes
    // inline GLSL in ShaderEffect (`fragmentShader` is a .qsb URL), so the
    // grain is a texture instead.
    Image {
        anchors.fill: parent
        visible: root.acrylic
        source: "qrc:/app/qml/noise.png"
        fillMode: Image.Tile
        opacity: Theme.acrylic.noise
        smooth: false
    }

    // The tint/grain are children, so they paint over the root border (an
    // item's border is background); redraw it on top to keep callers' borders.
    Rectangle {
        anchors.fill: parent
        visible: root.border.width > 0
        color: "transparent"
        radius: root.radius
        border.width: root.border.width
        border.color: root.border.color
    }
}
