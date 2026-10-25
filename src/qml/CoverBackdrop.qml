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

// Album/artist ambience for the rectangular pages. The sharp square is
// contained at the upper centre (never cropped or stretched); the rest of the
// page is a vignette centred on that square instead of a blurred copy.
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
        color: Palette.surfaceContainerLowest
    }

    // Cross vignette: two transparent-to-dark gradients, centred on the
    // artwork. Their overlap darkens the corners without an extra image pass.
    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0.0; color: Qt.rgba(0, 0, 0, 0.34) }
            GradientStop { position: 0.34; color: "transparent" }
            GradientStop { position: 0.66; color: "transparent" }
            GradientStop { position: 1.0; color: Qt.rgba(0, 0, 0, 0.46) }
        }
    }

    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0.0; color: Qt.rgba(0, 0, 0, 0.28) }
            GradientStop { position: 0.30; color: "transparent" }
            GradientStop { position: 0.70; color: "transparent" }
            GradientStop { position: 1.0; color: Qt.rgba(0, 0, 0, 0.28) }
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
