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

// Songs segment: the indexed songs, loaded in pages (PATH-3). The first page
// loads up to the remembered cursor (one page of headroom) so the scroll
// memory still lands; pages append as the cursor approaches the loaded end,
// and the player queue is built independently in Rust by `playSong`.
MediaList {
    id: root

    title: "Songs"
    orderText: Backend.songOrder
    memoryKey: "songs"

    function requestPreload() {
        root.loading = true;
        if (Backend.offline) {
            Backend.preloadSongs();
            return;
        }
        root.startVirtual("songs", "", Backend.songOrder);
    }

    function requestFull(showLoading) {
        root.stopPreload();
        root._fullLoaded = true;
        root.loading = showLoading;
        Backend.scrollHydrate();
        if (Backend.offline) {
            Backend.requestAllSongs();
            return;
        }
        root.startVirtual("songs", "", Backend.songOrder);
    }

    // G lands on the real last row: the count is already virtual, so land
    // directly after the slot table has it.
    edgeJumpOverride: function(last, count) {
        if (count !== undefined && count > 1)
            root.land(Math.min(count, root.count) - 1);
        else
            root.land(last ? root.count - 1 : 0);
    }

    function applyOfflinePage(payload) {
        var data = JSON.parse(payload);
        if (data.offset === 0)
            root.setRows(data.rows);
        else
            root.appendRows(data.rows);
        root.loading = false;
    }

    onActivated: (item) => Backend.playSong(item.songId)

    // ARC-UI-1: track rows carry all four recycled Alt verbs.
    quickActionKind: "song"

    Component.onCompleted: Qt.callLater(root.start)

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "songs_page")
                root.applyOfflinePage(payload);
        }
        function onLibrary_changed() {
            if (!root._preloadOnly)
                root.reload();
        }
        function onSong_order_changed() {
            // The cursor-position reset rides the shared row reload.
            if (!root._preloadOnly)
                root.resetRowReload();
        }
    }
}
