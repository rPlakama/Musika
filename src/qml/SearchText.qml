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
import "SearchMatch.js" as Match

// ARC-UI-2 M2: the row text with Neovim-style `hlsearch` blocks. The root is
// a `Text` — a true `FadeText` drop-in, so every cell keeps its natural
// implicit sizing, elide and alignment — with the blocks and the matched
// glyphs redrawn as children: rectangles measured by `FontMetrics` with the
// same shaper as the painted text (SEARCHACTIONSTUDY §6, kerning exact), then
// the matched substrings painted over them in the highlight foreground, the
// actual `Search`/`CurSearch` contrast pair. The children die at the cell
// edge (`clip`), so a match under the ellipsis cannot leak.
Text {
    id: root

    /// The live `/` query ("" = no session, no blocks) and whether this row
    /// is the session's current candidate (`CurSearch`).
    property string query: ""
    property bool currentMatch: false
    /// FadeText's contract: fade in on creation (off for recycled delegates).
    property bool animated: true

    font.family: Theme.fonts.body

    NumberAnimation on opacity {
        running: root.animated && !Theme.reduceMotion
        from: 0
        to: 1
        duration: Theme.motion.quick
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
    }

    readonly property var spans: Match.spans(root.text, root.query)

    // A block (or redraw) for a match under the ellipsis dies at the edge.
    clip: true

    FontMetrics {
        id: fm
        font: root.font
    }

    // Where the glyphs actually start: alignment shifts the painted run.
    readonly property real _originX: {
        if (root.horizontalAlignment === Text.AlignRight)
            return root.width - root.paintedWidth;
        if (root.horizontalAlignment === Text.AlignHCenter)
            return (root.width - root.paintedWidth) / 2;
        return 0;
    }

    Repeater {
        model: root.spans

        delegate: Rectangle {
            radius: Theme.radius
            required property var modelData

            x: root._originX + fm.advanceWidth(root.text.substring(0, modelData.start))
            y: Math.max(0, (root.height - fm.height) / 2)
            width: fm.advanceWidth(root.text.substring(0, modelData.end))
                - fm.advanceWidth(root.text.substring(0, modelData.start))
            height: fm.height
            color: root.currentMatch ? Palette.primary : Palette.primaryContainer
        }
    }

    Repeater {
        model: root.spans

        delegate: Text {
            required property var modelData

            x: root._originX + fm.advanceWidth(root.text.substring(0, modelData.start))
            y: Math.max(0, (root.height - fm.height) / 2)
            text: root.text.substring(modelData.start, modelData.end)
            font: root.font
            color: root.currentMatch ? Palette.onPrimary : Palette.onPrimaryContainer
        }
    }
}
