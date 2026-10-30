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

// Albums segment: the indexed album list fed asynchronously from library.db.
// The list UI and cursor behaviour live in the shared `MediaList`; this
// wrapper owns the `albums` channel and maps payload rows, so visual and
// navigation changes land in both this tab and Songs at once.
MediaList {
    id: root

    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)

    property string artistFilter: ""

    RowModel {
        id: virtualAlbums
    }
    virtualModel: virtualAlbums

    title: "Albums"
    orderText: Backend.albumOrder
    memoryKey: "albums"
    quickActionKind: "album"

    function requestPreload() {
        root.loading = true;
        virtualAlbums.setSurface("albums");
        virtualAlbums.setOrder(Backend.albumOrder);
        virtualAlbums.setFilter(root.artistFilter);
        virtualAlbums.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
    }

    function requestFull(showLoading) {
        root.stopPreload();
        root._fullLoaded = true;
        root.loading = showLoading;
        virtualAlbums.setSurface("albums");
        virtualAlbums.setOrder(Backend.albumOrder);
        virtualAlbums.setFilter(root.artistFilter);
        virtualAlbums.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
    }

    function apply(payload) {
        if (root.virtualModel)
            return;
        root.setRows(JSON.parse(payload));
    }

    onActivated: item => root.albumActivated(item.albumId, item.title, item.artistName, item.year, item.tracks, item.cover, item.starred === true)

    onArtistFilterChanged: {
        root.resetView();
        reload();
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "albums")
                root.apply(payload);
        }
        function onLibrary_changed() {
            // A dormant head must not turn into a full reload during the
            // startup sync; activation will request the fresh full payload.
            if (!root._preloadOnly)
                root.reload();
        }
        function onAlbum_order_changed() {
            if (!root._preloadOnly)
                root.resetRowReload();
        }
    }
}
