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

    readonly property int pageSize: 300
    property int _nextOffset: 0
    property bool _allLoaded: false
    property bool _loadingPage: false
    property bool _edgeTargetBottom: false

    RowModel {
        id: virtualSongs
    }
    virtualModel: virtualSongs
    model: virtualSongs.ready ? virtualSongs : null

    // `G` on the Songs list lands on the true last row. The tail-page path
    // remains for any paged payload; a scope load arrives complete.
    edgeJumpOverride: function(last, count) { root.jumpToEdge(last, count); }

    title: "Songs"
    orderText: Backend.songOrder
    memoryKey: "songs"

    function requestPreload() {
        root.loading = true;
        root._loadingPage = false;
        virtualSongs.setSurface("songs");
        virtualSongs.setOrder(Backend.songOrder);
        virtualSongs.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
        root.finishVirtualReset();
    }

    function requestFull(showLoading) {
        root.stopPreload();
        root._fullLoaded = true;
        // A scope switch loads the whole Songs segment on the uncapped row
        // path: the list is complete before the cursor can reach its end, so
        // no page cap applies while switching between scopes. Library and
        // order reloads ride the same full read.
        root.loading = showLoading;
        root._allLoaded = true;
        root._loadingPage = false;
        root._edgeTargetBottom = false;
        root._nextOffset = 0;
        virtualSongs.setSurface("songs");
        virtualSongs.setOrder(Backend.songOrder);
        virtualSongs.ensure(0, Theme.rowBudget - 1);
        root.loading = false;
        root.finishVirtualReset();
        Backend.scrollHydrate();
    }

    function applyPage(payload) {
        if (root.virtualModel)
            return;
        var data = JSON.parse(payload);
        // A page from before a reload/order swap must not append to the new
        // model (only the first page may restart the list).
        if (data.offset !== 0 && data.offset !== root._nextOffset)
            return;
        var rows = data.rows;
        if (data.offset === 0) {
            // MediaList owns the loading state while it fills the model in
            // batches.
            root.setRows(rows);
        } else {
            root.appendRows(rows);
            root.loading = false;
        }
        root._nextOffset = data.next;
        root._allLoaded = data.has_more !== true;
        root._loadingPage = false;
        if (root.count > 0 && root.currentIndex < 0)
            root.currentIndex = 0;
        if (rows.length === 0 && data.has_more === true)
            root.loadMore();
        else if (!root._allLoaded && root.count > 0 && root.currentIndex >= root.count - 20)
            root.loadMore();
        if (root.virtualModel)
            return;
        if (root._edgeTargetBottom) {
            if (root._allLoaded) {
                root._edgeTargetBottom = false;
                root.land(root.count - 1);
            } else {
                root.loadMore(root.pageSize * 4);
            }
        }
    }

    function loadMore(limit) {
        if (root.virtualModel)
            return;
        if (root._allLoaded || root._loadingPage)
            return;
        root._loadingPage = true;
        var size = limit === undefined ? root.pageSize : Math.max(root.pageSize, limit);
        Backend.requestSongsPage(root._nextOffset, size);
    }

    /// `G` on a paged list: finish the tail, then land on the real last row.
    function jumpToEdge(last, count) {
        if (count !== undefined && count > 1) {
            root.land(Math.min(count, root.count) - 1);
            return;
        }
        if (!last) {
            root.land(0);
            return;
        }
        if (root._allLoaded) {
            root.land(root.count - 1);
            return;
        }
        root._edgeTargetBottom = true;
        root.loadMore(root.pageSize * 4);
    }

    onCursorMoved: (index) => {
        if (root._edgeTargetBottom && index < root.count - 1)
            root._edgeTargetBottom = false;
        if (index >= root.count - 20)
            root.loadMore();
    }

    onActivated: (item) => Backend.playSong(item.songId)

    // ARC-UI-1: track rows carry all four recycled Alt verbs.
    quickActionKind: "song"

    Component.onCompleted: Qt.callLater(root.start)

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "songs_page")
                root.applyPage(payload);
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
