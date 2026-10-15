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

// UI-48: shared full-bleed row highlight, used by the search columns and the
// fullscreen queue. One block layer per row, overflowing the row bounds by
// `overflow` on both sides, so the cursor/hover fill reads edge-to-edge
// instead of as a floating stripe. The owner must clip a stage wider than the
// list (the rows themselves must not clip it):
//
//     RowHighlight { current: row.current; hovered: rowHover.hovered; overflow: bleed }
//
// `active` is the SearchColumn contract: a cursor in an unfocused column keeps
// a dim accent tick instead of the fill. Behaviour is colour-only with the
// shared `quick` fade, so nothing shifts layout.
Rectangle {
    id: root

    /// The row owns the cursor.
    property bool current: false
    /// The owning surface has focus (an unfocused cursor only shows the tick).
    property bool active: true
    /// Mouse is over the row.
    property bool hovered: false
    /// VI-4: the row is inside the visual selection.
    property bool selected: false
    /// How far the block bleeds past the row on each side.
    property real overflow: 0

    anchors.left: parent.left
    anchors.right: parent.right
    anchors.top: parent.top
    anchors.bottom: parent.bottom
    anchors.leftMargin: -root.overflow
    anchors.rightMargin: -root.overflow
    color: root.selected || (root.current && root.active)
        ? Palette.secondaryContainer
        : (root.hovered ? Palette.surfaceContainerHigh : "transparent")

    Behavior on color {
        ColorAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
    }

    // Inactive-column cue: a dim accent tick on the block.
    Rectangle {
        visible: root.current && !root.active
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: Math.max(2, root.height * 0.025)
        color: Palette.primary
        opacity: 0.35
    }
}
