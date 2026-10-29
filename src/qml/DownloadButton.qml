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

// ARC-UI-6: the one download-state visual, reused wherever a download
// function exists (`RowMarks`, ArtistEntry's chip, the fullscreen keybar
// chip). Three states, all optional:
//
//   idle    — the outline glyph (`showIdle`) when neither of the below
//   fetching— the shared loading loop, spinning; on stop it settles back
//             to the nearest full turn, so a finished glyph never keeps a
//             half-spin angle (the sync button's pattern)
//   done    — the filled glyph in the accent tint
//
//     DownloadButton {
//         base: parent.height * 0.42
//         fetching: root.downloading
//         done: root.cached
//         showIdle: true
//     }
Item {
    id: root

    /// The glyph's box.
    required property real base
    property bool fetching: false
    property bool done: false
    /// Draw the outline glyph when neither fetching nor done.
    property bool showIdle: false

    property color fetchingColor: Palette.onSurfaceVariant
    property color doneColor: Palette.tertiary
    property color idleColor: Palette.onSurfaceVariant

    implicitWidth: root.base
    implicitHeight: root.base

    Icon {
        id: glyph
        anchors.centerIn: parent
        width: Math.min(root.width, root.height)
        height: width
        visible: root.fetching || root.done || root.showIdle
        name: root.fetching
            ? "loading-loop"
            : (root.done ? "baseline-download" : "outline-download")
        symbolColor: root.fetching
            ? root.fetchingColor
            : (root.done ? root.doneColor : root.idleColor)
        transformOrigin: Item.Center

        NumberAnimation {
            id: spin
            target: glyph
            property: "rotation"
            from: 0
            to: 360
            duration: Theme.motion.spin
            loops: Animation.Infinite
            running: root.fetching && !Theme.reduceMotion
            onRunningChanged: if (!running) settle.restart()
        }

        // The stop lands the glyph upright: animate the leftover angle to
        // the nearest full turn (visually the start orientation).
        Anim {
            id: settle
            target: glyph
            property: "rotation"
            duration: Theme.motion.micro
            to: Math.round(glyph.rotation / 360) * 360
        }
    }
}
