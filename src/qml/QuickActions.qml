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
// `albumId` — or `album(item, cached)` — album rows/cards carry `albumId`;
// on an album the Go to Album slot is free (Enter already opens it), so it
// carries the container Download instead.
QtObject {
    /// Track row: all four verbs live. The heart reads the row's own state —
    /// already favourited shows heart-minus (the verb unstars), otherwise
    /// heart-add-fill (ARC-I-4).
    function song(item) {
        var fav = item.starred === true;
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("song", item.songId) }
            },
            south: {
                label: fav ? "Unfavorite" : "Favorite",
                icon: fav
                    ? "qrc:/app/icons/heart-minus.svg"
                    : "qrc:/app/icons/heart-add-fill.svg",
                favorite: !fav,
                invoke: function() { Backend.altFavorite("song", item.songId, !fav) }
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

    /// Album row/card: Go to Album is the page Enter already opens, so the
    /// east quadrant carries the container download toggle instead. `cached`
    /// comes from the caller's mark window (`Backend.cachedAlbumIds`): an
    /// album already holding its content removes it on the call (the
    /// ArtistEntry chip's toggle); anything else fires `downloadAlbum`
    /// (idempotent when a pass already covers it).
    function album(item, cached, preview) {
        var fav = item.starred === true;
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("album", item.albumId) }
            },
            south: {
                label: fav ? "Unfavorite" : "Favorite",
                icon: fav
                    ? "qrc:/app/icons/heart-minus.svg"
                    : "qrc:/app/icons/heart-add-fill.svg",
                favorite: !fav,
                // UI-60: the rating rides the favourite slot — Shift-J/K
                // step it while the overlay aims here.
                rating: Number(item.rating) || 0,
                rate: function(level) { Backend.rateAlbum(item.albumId, level) },
                // UI-60: the host.s local value drives the star colour
                // immediately; only persistence waits for the server.
                preview: preview,
                invoke: function() { Backend.altFavorite("album", item.albumId, !fav) }
            },
            west: {
                label: "Go to Artist",
                icon: "qrc:/app/icons/person-walking-svgrepo-com.svg",
                invoke: function() { Backend.altGoToArtist(item.albumId) }
            },
            east: cached === true ? {
                label: "Remove Download",
                icon: "qrc:/app/icons/baseline-download.svg",
                invoke: function() { Backend.downloadAlbum(item.albumId, false) }
            } : {
                label: "Download",
                icon: "qrc:/app/icons/outline-download.svg",
                invoke: function() { Backend.downloadAlbum(item.albumId, true) }
            }
        };
    }

    /// VI-4: with a visual selection active the range verbs apply to the
    /// whole selection — Add to Queue and Favorite. `starred` is the flags
    /// array paired with `ids` (Selection.starred): when every row is already
    /// favourited the heart wears heart-minus and unstars the range, otherwise
    /// it wears heart-add-fill and favourites only the rows that are not —
    /// mixed selections converge, never flip backwards (ARC-I-4). The go-to
    /// verbs stay single-row disabled; the free east quadrant carries the
    /// selection Download toggle (`downloadSelection`: songs explicit,
    /// albums — and every album of a selected artist — by container rule;
    /// a selection already holding its content removes instead).
    function forSelection(kind, ids, starred) {
        var all = ids.length > 0;
        for (var i = 0; i < ids.length; ++i)
            all = all && (starred && starred[i] === true);
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.queueAddSelection(kind, JSON.stringify(ids)) }
            },
            south: {
                label: all ? "Unfavorite" : "Favorite",
                icon: all
                    ? "qrc:/app/icons/heart-minus.svg"
                    : "qrc:/app/icons/heart-add-fill.svg",
                favorite: !all,
                invoke: function() {
                    var want = !all;
                    for (var i = 0; i < ids.length; ++i)
                        if ((starred && starred[i] === true) !== want)
                            Backend.altFavorite(kind, ids[i], want);
                }
            },
            west: {
                label: "Go to Artist",
                icon: "qrc:/app/icons/person-walking-svgrepo-com.svg",
                disabled: true
            },
            east: {
                label: "Download",
                icon: "qrc:/app/icons/outline-download.svg",
                invoke: function() { Backend.downloadSelection(kind, JSON.stringify(ids)) }
            }
        };
    }

    /// Destructive maintenance verbs (the SuperiorBar trashcan): the quadrant
    /// picks what to wipe. `sticky` keeps the overlay armed after a mouse
    /// click; the Alt-hold path passes false so releasing over neutral
    /// cancels and releasing over a quadrant fires it.
    function wipe(sticky) {
        return {
            sticky: sticky === true,
            north: {
                label: "Wipe everything!",
                icon: "qrc:/app/icons/delete-outline.svg",
                invoke: function() { Backend.wipeDatabases("all") }
            },
            south: {
                label: "Wipe albums!",
                icon: "qrc:/app/icons/album.svg",
                invoke: function() { Backend.wipeDatabases("library") }
            },
            east: {
                label: "Wipe indexes!",
                icon: "qrc:/app/icons/search-right-1505-svgrepo-com.svg",
                invoke: function() { Backend.wipeDatabases("indexes") }
            },
            west: {
                label: "Wipe covers!",
                icon: "qrc:/app/icons/album-2-svgrepo-com.svg",
                invoke: function() { Backend.wipeDatabases("covers") }
            }
        };
    }

    /// Artist hit (search): queue the whole artist and favourite it; both
    /// jumps stay open ("—") — the artist page is the row's Enter already.
    function artist(item, preview) {
        var fav = item.starred === true;
        return {
            north: {
                label: "Add to Queue",
                icon: "qrc:/app/icons/baseline-playlist-add.svg",
                invoke: function() { Backend.altAddToQueue("artist", item.artistId) }
            },
            south: {
                label: fav ? "Unfavorite" : "Favorite",
                icon: fav
                    ? "qrc:/app/icons/heart-minus.svg"
                    : "qrc:/app/icons/heart-add-fill.svg",
                favorite: !fav,
                // UI-60: the rating rides the favourite slot — Shift-J/K
                // step it while the overlay aims here.
                rating: Number(item.rating) || 0,
                rate: function(level) { Backend.rateArtist(item.artistId, level) },
                // UI-60: local-first fill; persistence is debounced.
                preview: preview,
                invoke: function() { Backend.altFavorite("artist", item.artistId, !fav) }
            },
            west: null,
            east: null
        };
    }
}
