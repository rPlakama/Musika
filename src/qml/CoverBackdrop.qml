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

// Album/artist ambience for the rectangular pages. The sharp square is
// contained at the upper centre (never cropped or stretched); the rest of the
// page is the same cover cropped edge-to-edge, blurred into its main colours.
Item {
    id: root

    property url source

    /// True once the covering image is decoded; hosts use this for their wash.
    readonly property bool ready: fill.ready

    /// The largest square that fits the page; the cover grows/shrinks with it.
    readonly property real coverSide: Math.min(width, height)

    visible: root.ready
    clip: true

    // Opaque base: blur edges and transparent padding blend into the surface.
    Rectangle {
        anchors.fill: parent
        color: Palette.surfaceContainerLowest
    }

    // Edge-to-edge bleed: cropped (the crop is hidden by the blur) and
    // desaturated only by the theme's acrylic recipe.
    CoverImage {
        id: fill
        anchors.fill: parent
        source: root.source
        // Half-res decode: the blur hides the difference and the page does not
        // pay for a second full-resolution image.
        sizeFactor: 0.5
        fillMode: Image.PreserveAspectCrop
        opacity: Theme.acrylic.enabled ? 1 : 0
        layer.enabled: fill.ready && Theme.acrylic.enabled
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: Theme.acrylic.blur
            blurMax: Theme.acrylic.blurMax
            saturation: 1.0
            autoPaddingEnabled: false
        }
    }

    // The artwork itself: top-centre, aspect-fit in the largest page square.
    CoverImage {
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.coverSide
        height: root.coverSide
        source: root.source
        fillMode: Image.PreserveAspectFit
    }
}
