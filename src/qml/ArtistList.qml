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

    title: "Artists"
    roundedArt: true
    orderText: Backend.artistOrder
    memoryKey: "artists"
    // UI-60: artist rows carry the favourite + rating verbs in the Alt
    // overlay (the rating rides the south slot).
    quickActionKind: "artist"

    function reload() {
        root.loading = true;
        Backend.requestArtists();
    }

    function apply(payload) {
        var parsed = JSON.parse(payload);
        var rows = [];
        for (var i = 0; i < parsed.length; i++) {
            var artist = parsed[i];
            var count = artist.album_count;
            rows.push({
                title: artist.name,
                subtitle: "",
                meta: count + (count === 1 ? " album" : " albums"),
                cover: artist.cover ? artist.cover : "",
                artistId: artist.id,
                artistName: artist.name,
                artistCover: artist.cover ? artist.cover : "",
                songId: "",
                albumId: "",
                starred: artist.starred === true,
                rating: Number(artist.rating) || 0
            });
        }
        root.setRows(rows);
        root.loading = false;
    }

    onActivated: (item) => root.artistActivated(item.artistId, item.artistName, item.artistCover)

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artists")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onArtist_order_changed() {
            // The cursor-position reset rides the shared row reload.
            root.resetRowReload();
        }
    }
}
