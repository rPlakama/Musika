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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import QtQuick
import musika

// ARC-UI-1: the four fixed Alt-hold verbs, one recycled module for every
// surface (Songs/Albums lists, ArtistEntry cards and the fullscreen queue).
// Hosts map their row item with `song(item)` — track rows carry `songId` and
// `albumId` — or `album(item)` — album rows/cards carry `albumId`; on an
// album the Go to Album slot stays open ("—"), since Enter already opens it.
QtObject {
    /// Track row: all four verbs live.
    function song(item) {
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("song", item.songId) }
            },
            south: {
                label: "Favorite",
                icon: "qrc:/app/icons/baseline-favorite.svg",
                invoke: function() { Backend.altFavorite("song", item.songId) }
            },
            west: {
                label: "Go to Artist",
                icon: "qrc:/app/icons/person-walking-svgrepo-com.svg",
                invoke: function() { Backend.altGoToArtist(item.albumId) }
            },
            east: {
                label: "Go to Album",
                icon: "qrc:/app/icons/album.svg",
                invoke: function() { Backend.altGoToAlbum(item.songId) }
            }
        };
    }

    /// Album row/card: Go to Album is the page Enter already opens.
    function album(item) {
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("album", item.albumId) }
            },
            south: {
                label: "Favorite",
                icon: "qrc:/app/icons/baseline-favorite.svg",
                invoke: function() { Backend.altFavorite("album", item.albumId) }
            },
            west: {
                label: "Go to Artist",
                icon: "qrc:/app/icons/person-walking-svgrepo-com.svg",
                invoke: function() { Backend.altGoToArtist(item.albumId) }
            },
            east: null
        };
    }

    /// Artist hit (search): queue the whole artist and favourite it; both
    /// jumps stay open ("—") — the artist page is the row's Enter already.
    function artist(item) {
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("artist", item.artistId) }
            },
            south: {
                label: "Favorite",
                icon: "qrc:/app/icons/baseline-favorite.svg",
                invoke: function() { Backend.altFavorite("artist", item.artistId) }
            },
            west: null,
            east: null
        };
    }
}
