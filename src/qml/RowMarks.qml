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
// every track of it is cached) followed by the favourite heart and, on
// album/artist rows only, its UI-60 rating count (songs keep the heart
// alone). Both marks render only for the rows inside the host viewport
// (`firstVisible`/`lastVisible`), and nothing renders when neither applies
// (the zero-alpha box collapses), so hosts only anchor the block and size it
// with `base` (the heart's box; the download glyph is 1.2× for even ink).
Rectangle {
    id: root

    /// The host's cursor (the selector): kept for fallback and identity.
    required property int cursor
    /// The visible band the host is rendering. Marks exist only inside it.
    property int firstVisible: -1
    property int lastVisible: -1
    /// The row's index and identity — album rows carry `albumId` only.
    required property int index
    property string songId: ""
    property string albumId: ""
    property bool starred: false
    /// UI-60: the row's user rating (0..=5, 0 = unrated). The count badge
    /// rides the favourite mark and stays visible even when unstarred —
    /// only on rows that own a rating (albums/artists), never on songs.
    property int rating: 0
    property bool showRating: false
    /// The heart's box; the download glyph uses 1.2× of it. The block
    /// scales the host's base once, here (+8%).
    required property real base
    readonly property real box: root.base * 1.08

    // Only rows inside the host viewport may paint a mark. Before the host
    // has a band (count/height not laid out yet) keep the row cheap.
    readonly property bool windowed: root.firstVisible >= 0
        && root.lastVisible >= root.firstVisible
        && root.index >= root.firstVisible
        && root.index <= root.lastVisible
    /// The row's fetch is running (queued/active in the ledger).
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
    /// UI-60: the rating count exists only for a real level.
    readonly property bool ratingMark: root.windowed && root.showRating
        && root.rating > 0
    /// Keep the button subtree alive only when it has something to paint.
    readonly property bool needsMarks: root.visible
        && (root.active || root.cached || root.starred || root.ratingMark)

    // UI-60: one zero-alpha, content-sized box owns the symbols; every mark
    // is vertically centred against the same Row axis. The box takes the
    // loader's intrinsic size and centres it; the mark buttons are created
    // lazily so unmarked rows do not carry their object trees.
    color: "transparent"
    width: markLoader.item ? markLoader.item.implicitWidth : 0
    height: markLoader.item ? markLoader.item.implicitHeight : 0

    Loader {
        id: markLoader
        anchors.centerIn: parent
        active: root.needsMarks
        sourceComponent: markSymbols
    }

    Component {
        id: markSymbols

        Row {
            spacing: root.box * 0.25

            Rectangle {
                radius: Theme.radius
                visible: root.active || root.cached
                anchors.verticalCenter: parent.verticalCenter
                width: root.box * 1.2
                height: width
                color: "transparent"

                // ARC-UI-6: the shared download visual.
                DownloadButton {
                    anchors.centerIn: parent
                    base: parent.width
                    fetching: root.active
                    done: root.cached
                }
            }

            // UI-60: the shared favourite button owns the heart + count.
            FavoriteButton {
                variant: "row"
                unit: root.box * 1.2
                anchors.verticalCenter: parent.verticalCenter
                starred: root.starred
                rating: root.rating
                showGlyph: root.windowed && root.starred
                showCount: root.ratingMark
            }
        }
    }
}
