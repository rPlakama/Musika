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

// The reusable shuffle toggle (VI-7's sibling): a press Fisher–Yates
// shuffles the tab's whole source into the playback queue in one load
// (nothing appends mid-session, so the queue rows never republish); a second
// press disables it. The tint tracks the backend's state, so the chip is a
// faithful read of "shuffling this source".
//
//     ShuffleChip { kind: "songs" }   // or "albums"; "" hides the chip
Item {
    id: root

    /// The backend source key: "songs" | "albums" | "" (hidden).
    required property string kind
    /// The glyph's box.
    property real unit: 24
    /// The Tab stop's ring (the panel's header scope).
    property bool focused: false

    visible: root.kind.length > 0
    implicitWidth: root.unit
    implicitHeight: root.unit

    readonly property bool active: Backend.shuffling
        && Backend.shuffleKind === root.kind

    ScopeRing {
        shown: root.focused
    }

    Icon {
        anchors.centerIn: parent
        width: root.unit
        height: width
        name: "outline-shuffle"
        symbolColor: root.active
            ? Palette.primary
            : (chipHover.hovered ? Palette.onSurface : Palette.onSurfaceVariant)

        Behavior on symbolColor {
            ColorAnimation {
                duration: Theme.motion.quick
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }
    }

    HoverHandler {
        id: chipHover
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        onTapped: Backend.toggleShuffle(root.kind)
    }
}
