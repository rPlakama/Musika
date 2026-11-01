// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

import QtQuick
import musika

// One persistent viewport for every library scope. Scope changes swap the
// data source and row policy; they never instantiate another ListView.
MediaList {
    id: root

    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)
    signal artistActivated(string artistId, string artistName, string artistCover)
    signal playlistActivated(string playlistId, string title)

    property string scope: "albums"

    readonly property bool isAlbums: root.scope === "albums"
    readonly property bool isSongs: root.scope === "songs"
    readonly property bool isArtists: root.scope === "artists"
    readonly property bool isPlaylists: root.scope === "playlists"

    title: root.isSongs ? "Songs"
        : root.isArtists ? "Artists"
        : root.isPlaylists ? "Playlists" : "Albums"
    orderText: root.isSongs ? Backend.songOrder
        : root.isArtists ? Backend.artistOrder
        : root.isAlbums ? Backend.albumOrder : ""
    memoryKey: root.scope
    quickActionKind: root.isSongs ? "song"
        : root.isArtists ? "artist"
        : root.isAlbums ? "album" : "playlist"
    roundedArt: root.isArtists
    showRowCover: !root.isPlaylists
    selectable: !root.isPlaylists
    emptyText: root.isPlaylists
        ? "No playlists yet — drop .m3u files in the playlists folder"
        : "Nothing indexed yet — run a sync"

    edgeJumpOverride: function(last, count) {
        if (root.isSongs && count !== undefined && count > 1)
            root.land(Math.min(count, root.count) - 1);
        else
            root.land(last ? root.count - 1 : 0);
    }

    /// Swap scopes while preserving both remembered cursor/offset pairs.
    function setScope(nextScope) {
        if (root.scope === nextScope)
            return;
        root.rememberView();
        root.clearSelection();
        root.scope = nextScope;
        root._fullLoaded = false;
        root._preloaded = false;
        root._preloadOnly = false;
        root._restored = false;
        root.resetView();
        root.reload();
    }

    function requestPreload() {
        root.loading = true;
        if (root.isPlaylists) {
            Backend.requestPlaylists();
            return;
        }
        if (Backend.offline) {
            if (root.isAlbums)
                Backend.preloadAlbums("");
            else if (root.isSongs)
                Backend.preloadSongs();
            else
                Backend.preloadArtists();
            return;
        }
        root.startVirtual(root.scope, "", root._order())
    }

    function requestFull(showLoading) {
        root.stopPreload();
        root._fullLoaded = true;
        root.loading = showLoading;
        if (root.isPlaylists) {
            Backend.requestPlaylists();
            return;
        }
        if (root.isSongs)
            Backend.scrollHydrate();
        if (Backend.offline) {
            if (root.isAlbums)
                Backend.requestAlbums("");
            else if (root.isSongs)
                Backend.requestAllSongs();
            else
                Backend.requestArtists();
            return;
        }
        root.startVirtual(root.scope, "", root._order())
    }

    function _order() {
        return root.isAlbums ? Backend.albumOrder
            : root.isSongs ? Backend.songOrder
            : root.isArtists ? Backend.artistOrder : "";
    }

    function applyAlbums(payload) {
        if (root.isAlbums && !root._virtualMode)
            root.setRows(JSON.parse(payload));
    }

    function applyArtists(payload) {
        if (root.isArtists && !root._virtualMode)
            root.setRows(JSON.parse(payload));
    }

    function applyPlaylists(payload) {
        if (!root.isPlaylists)
            return;
        root._virtualMode = false;
        root._virtualCount = 0;
        root.setRows(JSON.parse(payload));
        root.loading = false;
    }

    function applySongsPage(payload) {
        if (!root.isSongs)
            return;
        var data = JSON.parse(payload);
        if (data.offset === 0)
            root.setRows(data.rows);
        else
            root.appendRows(data.rows);
        root.loading = false;
    }

    onActivated: (item) => {
        if (root.isAlbums)
            root.albumActivated(item.albumId, item.title, item.artist,
                item.year, item.tracks, item.cover, item.starred === true);
        else if (root.isSongs)
            Backend.playSong(item.songId);
        else if (root.isArtists)
            root.artistActivated(item.artistId, item.artistName, item.artistCover);
        else if (root.isPlaylists)
            root.playlistActivated(item.playlistId, item.title);
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "albums")
                root.applyAlbums(payload);
            else if (channel === "artists")
                root.applyArtists(payload);
            else if (channel === "playlists")
                root.applyPlaylists(payload);
            else if (channel === "songs_page")
                root.applySongsPage(payload);
        }
        function onLibrary_changed() {
            if (!root._preloadOnly)
                root.reload();
        }
        function onAlbum_order_changed() {
            if (root.isAlbums && !root._preloadOnly)
                root.resetRowReload();
        }
        function onSong_order_changed() {
            if (root.isSongs && !root._preloadOnly)
                root.resetRowReload();
        }
        function onArtist_order_changed() {
            if (root.isArtists && !root._preloadOnly)
                root.resetRowReload();
        }
    }
}
