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

// Songs segment: all indexed songs. The list UI and cursor behaviour live in
// the shared `MediaList` (see AlbumList); this wrapper owns the `songs`
// channel and maps payload rows. Enter (or click) plays the selected song.
MediaList {
    id: root

    title: "Songs"
    orderText: Backend.songOrder
    memoryKey: "songs"

    function reload() {
        root.loading = true;
        Backend.requestSongs();
    }

    function apply(payload) {
        var parsed = JSON.parse(payload);
        var rows = [];
        for (var i = 0; i < parsed.length; i++) {
            var song = parsed[i];
            var artist = song.artist ? song.artist : "";
            var album = song.album ? song.album : "";
            rows.push({
                title: song.title,
                subtitle: artist + (album.length > 0 ? " · " + album : ""),
                meta: Theme.formatTime(song.duration),
                cover: song.cover ? song.cover : "",
                songId: song.id,
                albumId: song.album_id ? song.album_id : "",
                starred: song.starred === true
            });
        }
        root.setRows(rows);
        root.loading = false;
    }

    onActivated: (item) => Backend.playSong(item.songId)

    // ARC-UI-1: track rows carry all four recycled Alt verbs.
    quickActionKind: "song"

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "songs")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onSong_order_changed() {
            root.reload();
        }
    }
}
