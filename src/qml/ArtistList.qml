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

// Artists segment: all indexed artists. The list structure (header, cursor,
// gg/G, selection stage, gutters) is the shared `MediaList`, same as the
// Albums/Songs entries; this wrapper owns the `artists` channel and maps the
// payload. ArtistEntry is untouched. Shift-H/L cycle the Name/Albums order.
MediaList {
    id: root

    signal artistActivated(string artistId, string artistName, string artistCover)

    RowModel {
        id: virtualArtists
    }
    virtualModel: virtualArtists

    title: "Artists"
    roundedArt: true
    orderText: Backend.artistOrder
    memoryKey: "artists"
    // UI-60: artist rows carry the favourite + rating verbs in the Alt
    // overlay (the rating rides the south slot).
    quickActionKind: "artist"

    function requestPreload() {
        root.loading = true;
        virtualArtists.setSurface("artists");
        virtualArtists.setOrder(Backend.artistOrder);
        virtualArtists.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
    }

    function requestFull(showLoading) {
        root.stopPreload();
        root._fullLoaded = true;
        root.loading = showLoading;
        virtualArtists.setSurface("artists");
        virtualArtists.setOrder(Backend.artistOrder);
        virtualArtists.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
    }

    function apply(payload) {
        if (root.virtualModel)
            return;
        root.setRows(JSON.parse(payload));
    }

    onActivated: (item) => root.artistActivated(item.artistId, item.artist, item.artistCover)

    Component.onCompleted: Qt.callLater(root.start)

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artists")
                root.apply(payload);
        }
        function onLibrary_changed() {
            if (!root._preloadOnly)
                root.reload();
        }
        function onArtist_order_changed() {
            // The cursor-position reset rides the shared row reload.
            if (!root._preloadOnly)
                root.resetRowReload();
        }
    }
}
