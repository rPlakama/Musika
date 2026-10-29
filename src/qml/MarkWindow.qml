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

// The one download-mark window and rating patcher. A surface only exposes
// the rows it can paint (the rendered band, or the cursor before layout) and
// this object:
//
//   - asks the backend for those rows' download state, coalesced to one
//     request per event-loop turn (`force` bypasses the coalescing for the
//     `rowReload` reconciliation);
//   - patches every matching album/artist row when `ratingChanged` lands.
//
// Hosts bind `model`, the visible band and the cursor, then set `songs` /
// `albums` for the row kinds they render. No host builds its own id arrays.
QtObject {
    id: root

    /// The list model the surface paints (ListModel, not a JS array).
    property var model: null
    /// The rendered band (`ScrollContent.firstVisibleIndex`/`lastVisibleIndex`).
    property int firstVisible: -1
    property int lastVisible: -1
    /// The host cursor, the fallback window before the band is laid out.
    property int cursor: -1
    /// Which row kinds this surface owns.
    property bool songs: true
    property bool albums: false

    property int count: root.model ? root.model.count : 0
    readonly property bool enabled: (root.songs || root.albums)
        && root.model !== null && root.model.get !== undefined
        && root.model.count !== undefined

    property bool _forcePending: false

    // One request per event-loop turn: a cursor move and the band change it
    // causes share a single backend call. (A QtObject has no default child
    // list, so the non-visual objects live in object properties.)
    property Timer coalesce: Timer {
        interval: 0
        repeat: false
        onTriggered: root._flush()
    }

    property Connections backend: Connections {
        target: Backend
        function onRowReload() {
            root.request(true);
        }
        function onRatingChanged(kind, id, rating) {
            root.patchRating(kind, id, rating);
        }
    }

    onModelChanged: root.request()
    onFirstVisibleChanged: root.request()
    onLastVisibleChanged: root.request()
    onCursorChanged: root.request()
    onCountChanged: root.request()

    /// The rendered band, or the cursor until the first real layout.
    function _window() {
        var lo = root.firstVisible;
        var hi = root.lastVisible;
        if (lo < 0 || hi < lo) {
            lo = Math.max(0, root.cursor);
            hi = lo;
        }
        if (lo < 0 || lo >= root.count)
            return null;
        hi = Math.min(root.count - 1, hi);
        if (hi < lo)
            return null;
        return { lo: lo, hi: hi };
    }

    /// Ask for this surface's rows' download state. `force` refreshes past the
    /// backend's dedupe (a row reload); a plain request is coalesced.
    function request(force) {
        if (!root.enabled)
            return;
        if (force === true) {
            root._forcePending = true;
            coalesce.stop();
            root._flush();
            return;
        }
        coalesce.restart();
    }

    function _flush() {
        if (!root.enabled)
            return;
        var force = root._forcePending;
        root._forcePending = false;
        var window = root._window();
        if (!window)
            return;
        var songIds = [];
        var albumIds = [];
        for (var i = window.lo; i <= window.hi; ++i) {
            var row = root.model.get(i);
            if (!row)
                continue;
            // A row is one kind: a song row rides its song id even when it
            // also carries albumId (the old per-host rule), otherwise the
            // album-kind id.
            if (root.songs && row.songId)
                songIds.push(row.songId);
            else if (root.albums && row.albumId)
                albumIds.push(row.albumId);
        }
        if (songIds.length === 0 && albumIds.length === 0)
            return;
        if (force)
            Backend.refreshDownloads(JSON.stringify(songIds), JSON.stringify(albumIds));
        else
            Backend.requestDownloads(JSON.stringify(songIds), JSON.stringify(albumIds));
    }

    /// Patch a rated album/artist row in place (the models carry `rating`).
    function patchRating(kind, id, rating) {
        if (!root.enabled || !root.albums || !root.model || !root.model.setProperty)
            return;
        for (var i = 0; i < root.model.count; ++i) {
            var row = root.model.get(i);
            if (!row || row.rating === undefined)
                continue;
            if ((kind === "album" && row.albumId === id)
                    || (kind === "artist" && row.artistId === id))
                root.model.setProperty(i, "rating", rating);
        }
    }
}
