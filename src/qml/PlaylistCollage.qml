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

// Playlist ambience: a square mosaic of up to three distinct song covers.
// The panes split by the golden ratio: a 61.8% master pane and a 38.2% edge;
// with three covers the edge is split again at 61.8/38.2.
Item {
    id: root

    property var sources: []
    property real maxSide: Number.POSITIVE_INFINITY
    /// Extra rows below the art: the square never grows behind them.
    readonly property real coverSide: Math.min(width, height, maxSide)
    readonly property real phi: (1 + Math.sqrt(5)) / 2
    readonly property real large: 1 / root.phi
    readonly property real small: 1 - root.large
    readonly property real gap: Math.max(1, root.coverSide * 0.004)

    readonly property var covers: {
        var out = []
        for (var i = 0; i < root.sources.length && out.length < 3; ++i) {
            var source = root.sources[i]
            if (source && source.toString().length > 0)
                out.push(source)
        }
        return out
    }

    clip: true

    Rectangle {
        anchors.fill: parent
        color: "#000000"
    }

    Rectangle {
        id: mosaic
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.coverSide
        height: root.coverSide
        color: Palette.surfaceContainerHigh
        clip: true

        // One cover, or the placeholder when the playlist has no art.
        CoverImage {
            visible: root.covers.length < 2
            anchors.fill: parent
            source: root.covers.length > 0 ? root.covers[0] : ""
            fillMode: Image.PreserveAspectCrop
            sizeFactor: 1.25
        }

        // Master pane (left), also the top pane for the two-cover split.
        CoverImage {
            visible: root.covers.length >= 2
            anchors.left: parent.left
            anchors.top: parent.top
            width: root.covers.length >= 3
                ? parent.width * root.large - root.gap / 2
                : parent.width * root.large - root.gap / 2
            height: parent.height
            source: root.covers.length > 0 ? root.covers[0] : ""
            fillMode: Image.PreserveAspectCrop
            sizeFactor: 1.25
        }

        // Right/top pane.
        CoverImage {
            visible: root.covers.length >= 2
            anchors.right: parent.right
            anchors.top: parent.top
            width: parent.width * root.small - root.gap / 2
            height: root.covers.length >= 3
                ? parent.height * root.large - root.gap / 2
                : parent.height
            source: root.covers.length > 1 ? root.covers[1] : ""
            fillMode: Image.PreserveAspectCrop
            sizeFactor: 1.25
        }

        // Right/bottom pane, only for three or more covers.
        CoverImage {
            visible: root.covers.length >= 3
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            width: parent.width * root.small - root.gap / 2
            height: parent.height * root.small - root.gap / 2
            source: root.covers.length > 2 ? root.covers[2] : ""
            fillMode: Image.PreserveAspectCrop
            sizeFactor: 1.25
        }

        // Empty-playlist mark; invisible when at least one cover exists.
        AppText {
            anchors.centerIn: parent
            visible: root.covers.length === 0
            text: "♪"
            color: Palette.onSurfaceVariant
            font.pixelSize: Math.max(28, root.coverSide * 0.16)
        }
    }
}
