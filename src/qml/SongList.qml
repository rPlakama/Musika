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
import "ScrollMemory.js" as Memory

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
    property int _pageRequest: 0
    property bool _pageStarted: false
    property bool _pageStale: false
    property int _pageOffset: 0
    property int _pageRows: 0

    RowLoader {
        id: pageLoader
    }

    // `G` on the paged Songs list finishes the tail, then lands on the true
    // last row instead of the last loaded one.
    edgeJumpOverride: function(last, count) { root.jumpToEdge(last, count); }

    title: "Songs"
    orderText: Backend.songOrder
    memoryKey: "songs"

    function reload() {
        root.loading = true;
        root._allLoaded = false;
        root._loadingPage = true;
        root._edgeTargetBottom = false;
        root._nextOffset = 0;
        // UI-54: preload through the remembered cursor so `restore()` lands on
        // the same row; beyond that, pages stream in on demand.
        var saved = Memory.loadScroll(root.memoryKey);
        var at = root.currentIndex >= 0
            ? root.currentIndex
            : (saved && saved.index >= 0 ? saved.index : 0);
        var limit = Math.max(root.pageSize, at + root.pageSize);
        Backend.requestSongsPage(0, limit);
    }

    function applyPage(payload) {
        root._pageRequest++;
        root._pageStarted = false;
        root._pageStale = false;
        root._pageRows = 0;
        pageLoader.load("songs-page", payload);
    }

    function _finishPage(meta) {
        root._nextOffset = meta.next;
        root._allLoaded = meta.has_more !== true;
        root._loadingPage = false;
        root.loading = false;
        if (root.count > 0 && root.currentIndex < 0)
            root.currentIndex = 0;
        if (root._pageRows === 0 && meta.has_more === true)
            root.loadMore();
        else if (!root._allLoaded && root.count > 0
                && root.currentIndex >= root.count - 20)
            root.loadMore();
        if (root._edgeTargetBottom) {
            if (root._allLoaded) {
                root._edgeTargetBottom = false;
                root.land(root.count - 1);
            } else {
                root.loadMore(root.pageSize * 4);
            }
        }
    }

    Connections {
        target: pageLoader
        function onBatch(requestId, kind, rows, done, meta) {
            if (requestId !== root._pageRequest)
                return;
            var first = !root._pageStarted;
            if (first) {
                root._pageStarted = true;
                root._pageOffset = meta.offset;
                if (meta.offset !== 0 && meta.offset !== root._nextOffset) {
                    root._pageStale = true;
                    root._loadingPage = false;
                    return;
                }
            }
            if (root._pageStale)
                return;
            root._pageRows += rows.length;
            if (root._pageOffset === 0 && first)
                root.setRows(rows);
            else
                root.appendRows(rows);
            if (done)
                root._finishPage(meta);
        }
        function onFailed(requestId, kind, message) {
            if (requestId === root._pageRequest) {
                root._loadingPage = false;
                root.loading = false;
                console.warn("songs page load failed: " + message);
            }
        }
    }

    function loadMore(limit) {
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

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "songs_page")
                root.applyPage(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onSong_order_changed() {
            // The cursor-position reset rides the shared row reload.
            root.resetRowReload();
        }
    }
}
