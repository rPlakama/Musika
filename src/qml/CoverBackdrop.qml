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

    // Radial vignette around the square: transparent at the artwork, darker
    // toward the page edges. Painted once per size, not per frame.
    Canvas {
        id: vignette
        anchors.fill: parent

        onPaint: {
            if (root.width <= 0 || root.height <= 0)
                return;
            var ctx = getContext("2d");
            ctx.clearRect(0, 0, root.width, root.height);
            var cx = root.width * 0.5;
            var cy = root.coverSide * 0.5;
            var inner = Math.max(1, root.coverSide * 0.32);
            var outer = Math.max(root.width, root.height) * 0.78;
            var gradient = ctx.createRadialGradient(cx, cy, inner, cx, cy, outer);
            gradient.addColorStop(0.0, "rgba(0,0,0,0)");
            gradient.addColorStop(0.55, "rgba(0,0,0,0.16)");
            gradient.addColorStop(1.0, "rgba(0,0,0,0.62)");
            ctx.fillStyle = gradient;
            ctx.fillRect(0, 0, root.width, root.height);
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
    }

    Connections {
        target: root
        function onCoverSideChanged() { vignette.requestPaint(); }
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
