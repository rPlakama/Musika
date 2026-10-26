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

// Album/artist ambience for the rectangular pages. A blurred, darkened copy
// of the artwork fills the surface; the sharp square stays contained at the
// upper centre (never cropped or stretched).
Item {
    id: root

    property url source

    /// True once the artwork is decoded; hosts use this for their wash.
    readonly property bool ready: cover.ready

    /// Optional ceiling for hosts that reserve a rows band below the art.
    property real maxSide: Number.POSITIVE_INFINITY

    /// The largest square that fits the page; the cover grows/shrinks with it.
    readonly property real coverSide: Math.min(width, height, maxSide)

    visible: root.ready
    clip: true

    Rectangle {
        anchors.fill: parent
        color: Palette.surfaceContainerLow
    }

    // Blurred cover fill, colourised with the second-darkest generated
    // surface tone, then the sharp artwork.
    CoverImage {
        id: fill
        anchors.fill: parent
        source: root.source
        sizeFactor: 0.5
        fillMode: Image.PreserveAspectCrop
        opacity: fill.ready ? 0.90 : 0
        layer.enabled: fill.ready
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: 0.8
            blurMax: 48
            saturation: 1.0
            colorizationColor: Palette.surfaceContainerLow
            colorization: 0.35
            autoPaddingEnabled: false
        }
    }

    // The artwork itself: top-centre, aspect-fit in the largest page square.
    CoverImage {
        id: cover
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.coverSide
        height: root.coverSide
        source: root.source
        fillMode: Image.PreserveAspectFit
    }
}
