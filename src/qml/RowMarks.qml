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
    /// The heart's box; the download glyph uses 1.2× of it. The block
    /// scales the host's base once, here (+8%).
    required property real base
    readonly property real box: root.base * 1.08

    spacing: root.box * 0.05

    // The window's reference is the selector; a not-yet-landed cursor
    // clamps to 0 (the same rule the hosts' request windows use).
    readonly property bool windowed: Math.abs(root.index - Math.max(0, root.cursor)) <= 15
    /// The row's fetch is running (queued/active in the ledger): the row
    /// shows the shared loading loop, spinning, instead of the filled mark.
    /// Album rows ride the same set through their own id — a container pass
    /// also lights the album's row.
    readonly property bool active: root.windowed
        && ((root.songId.length > 0
                && Backend.downloadingIds.indexOf(root.songId) !== -1)
            || (root.albumId.length > 0
                && Backend.downloadingIds.indexOf(root.albumId) !== -1))
    /// The card's cache state, or the track's own.
    readonly property bool cached: root.windowed
        && ((root.songId.length > 0
                && Backend.downloadedIds.indexOf(root.songId) !== -1)
            || (root.albumId.length > 0
                && Backend.cachedAlbumIds.indexOf(root.albumId) !== -1))

    Rectangle {
        visible: root.active || root.cached
        width: root.box * 1.2
        height: width
        color: "transparent"

        // ARC-UI-6: the shared download visual (loading while fetching,
        // filled when cached, spin settling upright).
        DownloadButton {
            anchors.centerIn: parent
            base: parent.width
            fetching: root.active
            done: root.cached
        }
    }

    Rectangle {
        visible: root.windowed && root.starred
        width: root.box * 1.2
        height: width
        color: "transparent"

        Icon {
            anchors.centerIn: parent
            width: root.box
            height: width
            name: "baseline-favorite"
            symbolColor: Theme.iconStrong
        }
    }
}
