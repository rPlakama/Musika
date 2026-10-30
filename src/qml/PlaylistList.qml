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

// ARC-44 M0: the Playlists segment — local m3u playlists indexed into
// `playlist.db` (Subsonic playlists join in M1, the entry page in M2). The
// list UI and cursor behaviour live in the shared `MediaList`; this wrapper
// owns the `playlists` channel and maps payload rows.
MediaList {
    id: root

    signal playlistActivated(string playlistId, string title)

    title: "Playlists"
    quickActionKind: "playlist"
    selectable: false
    memoryKey: "playlists"
    showRowCover: false
    emptyText: "No playlists yet — drop .m3u files in the playlists folder"

    function reload() {
        root.loading = true;
        Backend.requestPlaylists();
    }

    function apply(payload) {
        root.setRows(JSON.parse(payload));
    }

    onActivated: (item) => root.playlistActivated(item.playlistId, item.title)

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "playlists")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }
}
