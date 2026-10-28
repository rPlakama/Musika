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

    title: "Albums"
    orderText: Backend.albumOrder
    memoryKey: "albums"

    property bool _preloaded: false
    property bool _fullLoaded: false
    property bool _preloadOnly: false

    function start() {
        if (root._preloadOnly)
            root.preload();
        else
            root.activateTab();
    }

    function activateTab() {
        preloadTimer.stop();
        if (root._fullLoaded)
            return;
        root.requestFull(root.count === 0);
    }

    function preload() {
        if (root._preloaded)
            return;
        root._preloaded = true;
        preloadTimer.restart();
    }

    function requestPreload() {
        root.loading = true;
        Backend.preloadAlbums(root.artistFilter);
    }

    function reload() {
        root.requestFull(true);
    }

    function requestFull(showLoading) {
        preloadTimer.stop();
        root._fullLoaded = true;
        root.loading = showLoading;
        Backend.requestAlbums(root.artistFilter);
    }

    Timer {
        id: preloadTimer
        interval: 350
        repeat: false
        onTriggered: root.requestPreload()
    }

    function apply(payload) {
        root.setRows(JSON.parse(payload));
    }

    onActivated: (item) => root.albumActivated(item.albumId, item.title, item.artist, item.year, item.tracks, item.cover, item.starred === true)

    // ARC-UI-1: album rows keep three verbs live and the Go to Album slot
    // open (Enter already opens the album).
    quickActionKind: "album"

    Component.onCompleted: Qt.callLater(root.start)

    // UI-10: a new filter is a new list — reset before the payload so the
    // old cursor is not chased into the rebuilt rows.
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
            root.reload();
        }
        function onAlbum_order_changed() {
            // The cursor-position reset rides the shared row reload.
            root.resetRowReload();
        }
    }
}
