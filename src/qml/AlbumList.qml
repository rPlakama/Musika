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

    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)

    property string artistFilter: ""

    title: "Albums"
    orderText: Backend.albumOrder
    memoryKey: "albums"

    function reload() {
        root.loading = true;
        Backend.requestAlbums(root.artistFilter);
    }

    function apply(payload) {
        var parsed = JSON.parse(payload);
        var rows = [];
        for (var i = 0; i < parsed.length; i++) {
            var album = parsed[i];
            var artist = album.artist ? album.artist : "";
            var year = album.year ? String(album.year) : "";
            rows.push({
                title: album.title,
                subtitle: artist + (year.length > 0 ? " · " + year : ""),
                meta: String(album.song_count) + " tracks",
                cover: album.cover ? album.cover : "",
                albumId: album.id,
                artist: artist,
                year: year,
                tracks: String(album.song_count)
            });
        }
        root.setRows(rows);
        root.loading = false;
    }

    onActivated: (item) => root.albumActivated(item.albumId, item.title, item.artist, item.year, item.tracks, item.cover)

    // ARC-43/ARC-UI-1: Alt-hold quick actions for an album row — four fixed
    // verbs, minus Go to Album (Enter already opens it): that slot stays the
    // open "—".
    quickActions: function(item) {
        return {
            north: { label: "Add to Queue", invoke: function() { Backend.altAddToQueue("album", item.albumId) } },
            south: { label: "Favorite", invoke: function() { Backend.altFavorite("album", item.albumId) } },
            west: { label: "Go to Artist", invoke: function() { Backend.altGoToArtist(item.albumId) } },
            east: null
        };
    }

    Component.onCompleted: reload()

    onArtistFilterChanged: reload()

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
            root.reload();
        }
    }
}
