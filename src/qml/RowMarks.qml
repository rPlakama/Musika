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

// REC-UI-1: the one state-mark block every list row shares — the download
// glyph (the windowed download.db sets: the song's id, or the album's when
// every track of it is cached) followed by the favourite heart. Both marks
// render only inside the host cursor's ±15-row window (`cursor`), and
// nothing renders when neither applies (invisible boxes collapse in the
// Row), so hosts only anchor the block and size it with `base` (the heart's
// box; the download glyph is 1.2× for even ink).
//
//     RowMarks {
//         anchors.right: meta.left
//         cursor: root.currentIndex
//         index: row.index
//         songId: row.songId
//         albumId: row.albumId
//         starred: row.starred
//         base: meta.font.pixelSize * 0.8
//     }
Row {
    id: root

    /// The host's cursor (the selector): the rendering window's reference.
    required property int cursor
    /// The row's index and identity — album rows carry `albumId` only.
    required property int index
    property string songId: ""
    property string albumId: ""
    property bool starred: false
    /// The heart's box; the download glyph uses 1.2× of it.
    required property real base

    spacing: root.base * 0.05

    readonly property bool windowed: Math.abs(root.index - root.cursor) <= 15

    Rectangle {
        // The card's cache state, or the track's own.
        visible: root.windowed
            && ((root.songId.length > 0
                    && Backend.downloadedIds.indexOf(root.songId) !== -1)
                || (root.albumId.length > 0
                    && Backend.cachedAlbumIds.indexOf(root.albumId) !== -1))
        width: root.base * 1.2
        height: width
        color: "transparent"

        Icon {
            anchors.centerIn: parent
            width: parent.width
            height: width
            name: "baseline-download"
            symbolColor: Palette.tertiary
        }
    }

    Rectangle {
        visible: root.windowed && root.starred
        width: root.base * 1.2
        height: width
        color: "transparent"

        Icon {
            anchors.centerIn: parent
            width: root.base
            height: width
            name: "baseline-favorite"
            symbolColor: Theme.iconStrong
        }
    }
}
