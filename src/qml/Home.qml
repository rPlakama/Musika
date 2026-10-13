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
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Effects
import musika
import "Theme.js" as Theme

// Library shell: status bar on top, sidebar | main below it, player bar at the
// bottom.
//
// Focus model (vi-like): `activeRegion` is the cursor and its border stays lit
// at all times. Ctrl-W enters region mode: a niri-like blur + scrim dims the
// shell and the region titles show inside the selection squares, j/k cycles
// candidates, Enter accepts.
Item {
    id: root

    // 0 = sidebar, 1 = main, 2 = status (top), 3 = inferior bar.
    property int activeRegion: 1
    property int visualSelection: 1
    property bool visualMode: false
    property bool nowPlayingExpanded: false
    property int currentSegment: 0

    readonly property real barH: height * Theme.topBarRatio
    readonly property real playH: height * Theme.playerRatio
    readonly property real sideW: width * Theme.sidebarRatio
    readonly property real bodyH: height - barH - playH

    // View transitions (motion §3.1): fade in with decelerate, fade out with
    // accelerate. One factory reused by push/pop/replace.
    component EnterView: Transition {
        NumberAnimation {
            property: "opacity"; from: 0; to: 1
            duration: Theme.motion.standard
            easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.decelerate
        }
    }

    component ExitView: Transition {
        NumberAnimation {
            property: "opacity"; from: 1; to: 0
            duration: Theme.motion.quick
            easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.accelerate
        }
    }

    function rootFor(index) {
        switch (index) {
        case 1:
            return songListComponent;
        case 2:
            return artistListComponent;
        case 3:
            return playlistListComponent;
        case 4:
            return radioListComponent;
        default:
            return albumListComponent;
        }
    }

    function showSegment(index) {
        currentSegment = index;
        while (contentStack.depth > 1)
            contentStack.pop();
        contentStack.replace(rootFor(index));
        activeRegion = 1;
    }

    // Spatial neighbours of a region: 0 sidebar, 1 main, 2 status (top),
    // 3 player (bottom). Returns -1 when there is nothing in that direction.
    function regionInDirection(region, dir) {
        switch (region) {
        case 1: // main
            if (dir === "left")
                return 0;
            if (dir === "up")
                return 2;
            if (dir === "down")
                return 3;
            return -1;
        case 0: // sidebar
            if (dir === "right")
                return 1;
            if (dir === "up")
                return 2;
            if (dir === "down")
                return 3;
            return -1;
        case 2: // status
            return dir === "down" ? 1 : -1;
        case 3: // player
            return dir === "up" ? 1 : -1;
        default:
            return -1;
        }
    }

    function moveVisual(dir) {
        var target = regionInDirection(visualSelection, dir);
        if (target >= 0)
            visualSelection = target;
    }

    // Global fullscreen toggle (dispatched by the app-level handler before
    // screen/region routing, so it works from any region and overlay state).
    function toggleFullscreen() {
        // Fullscreen is the topmost overlay: region selection ends with it.
        if (!nowPlayingExpanded)
            visualMode = false;
        nowPlayingExpanded = !nowPlayingExpanded;
    }

    // Clipboard paste (VI-21): the text lands visibly in the search field so
    // the user can correct it; Enter plays the selection.
    function pasteSearch(text) {
        searchOverlay.openWith(text);
    }

    // Actions dispatched from the global key handler, scoped to the active
    // region: movement only ever affects the region the cursor is in.
    function handleAction(action) {
        // Command pump: the search overlay (and external IPC via `ipc.sock`)
        // both express this as `play_song_in_album <song-id>`.
        if (action.indexOf("play_song_in_album ") === 0) {
            Backend.playSongInAlbum(action.substring("play_song_in_album ".length));
            return;
        }

        // The floating search is modal: while open, all keys go to it.
        if (searchOverlay.opened) {
            searchOverlay.handleAction(action);
            return;
        }
        // The queue drawer is modal too.
        if (queueDrawer.opened) {
            queueDrawer.handleAction(action);
            return;
        }
        // Transport is global once the modals above have had their say:
        // the search overlay owns its input, so Space cannot leak through
        // (VI-18).
        if (action === "play_pause") {
            Backend.playerToggle();
            return;
        }
        // Ctrl-Q toggles the queue drawer from anywhere.
        if (action === "queue_toggle") {
            queueDrawer.toggle();
            return;
        }

        if (action === "visual_mode") {
            if (nowPlayingExpanded) {
                nowPlayingExpanded = false;
                return;
            }
            visualMode = !visualMode;
            visualSelection = activeRegion;
            return;
        }

        // Search is global scope: open the floating search from any region.
        if (action === "search") {
            searchOverlay.open();
            return;
        }
        // In visual mode, navigation selects a candidate region; Enter commits.
        if (visualMode) {
            if (action === "move_left")
                moveVisual("left");
            else if (action === "move_right")
                moveVisual("right");
            else if (action === "move_up")
                moveVisual("up");
            else if (action === "move_down")
                moveVisual("down");
            else if (action === "enter") {
                // Accept the candidate: land in it and return to normal mode.
                activeRegion = visualSelection;
                visualMode = false;
            } else if (action === "back") {
                visualMode = false;
            }
            return;
        }

        if (action === "back") {
            // Fullscreen is the topmost context: back leaves it first.
            if (nowPlayingExpanded) {
                nowPlayingExpanded = false;
                return;
            }
            if (contentStack.depth > 1) {
                contentStack.pop();
            }
            return;
        }

        // Fullscreen is modal for navigation: transport and pane keys belong
        // to the player view while it is open (Esc and Space stay global).
        if (nowPlayingExpanded) {
            nowPlaying.handleAction(action);
            return;
        }

        // Between the left bar and main, h/l move directionally.
        if (action === "move_left" && activeRegion === 1) {
            activeRegion = 0;
            return;
        }
        if (action === "move_right" && activeRegion === 0) {
            activeRegion = 1;
            return;
        }

        switch (activeRegion) {
        case 0:
            sideBar.handleAction(action);
            break;
        case 1:
            if (contentStack.currentItem && contentStack.currentItem.handleAction)
                contentStack.currentItem.handleAction(action);
            break;
        case 2:
            topBar.handleAction(action);
            break;
        case 3:
            player.handleAction(action);
            break;
        }
    }

    ColumnLayout {
        id: shell
        anchors.fill: parent
        spacing: 0

        // Niri-like backdrop blur while a modal surface is up (floating search
        // or the Ctrl-W scope overlay). Qt approximation of the niri config:
        // passes/offset -> blur + blurMax, saturation 0.6 == niri's 1.6x,
        // autoPadding off so the blur stays clipped to the window. MultiEffect
        // has no grain, so niri's `noise 0.03` is not reproduced.
        layer.enabled: root.visualMode || searchOverlay.opened
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: 0.5
            blurMax: 48
            saturation: 0.6
            autoPaddingEnabled: false
        }

        RightBar {
            id: topBar
            Layout.fillWidth: true
            Layout.preferredHeight: root.barH
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            SideBar {
                id: sideBar
                Layout.preferredWidth: root.sideW
                Layout.fillHeight: true

                onSegmentActivated: (index) => root.showSegment(index)
            }

            ColumnLayout {
                id: mainArea
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                // Search is global scope (Shift Space F); this is just the hint.
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.height * 0.045
                    color: Palette.surface

                    FadeText {
                        anchors.left: parent.left
                        anchors.leftMargin: root.width * 0.02
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Shift Space F for Search"
                        color: Palette.onSurfaceVariant
                        font.pixelSize: parent.height * 0.42
                    }

                    FadeText {
                        anchors.right: parent.right
                        anchors.rightMargin: root.width * 0.02
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Shift-H / Shift-L  --  Order by " + Backend.albumOrder
                        color: Palette.onSurfaceVariant
                        font.pixelSize: parent.height * 0.42
                    }
                }

                StackView {
                    id: contentStack
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    initialItem: albumListComponent

                    pushEnter: EnterView {}
                    pushExit: ExitView {}
                    popEnter: EnterView {}
                    popExit: ExitView {}
                    replaceEnter: EnterView {}
                    replaceExit: ExitView {}

                    Component {
                        id: albumListComponent
                        AlbumList {
                            onAlbumActivated: (albumId, title, artist, year, tracks, cover) => {
                                var meta = [];
                                if (year.length > 0)
                                    meta.push(year);
                                if (tracks.length > 0)
                                    meta.push(tracks + " songs");
                                contentStack.push(albumEntryComponent, {
                                    albumId: albumId,
                                    albumTitle: title,
                                    albumArtist: artist,
                                    albumCover: cover,
                                    albumMeta: meta.join("   ·   ")
                                });
                            }
                        }
                    }

                    Component {
                        id: songListComponent
                        SongList {}
                    }

                    Component {
                        id: artistListComponent
                        ArtistList {
                            onArtistActivated: (artist) => contentStack.replace(albumListComponent, { artistFilter: artist })
                        }
                    }

                    Component {
                        id: playlistListComponent
                        PlaceholderList {
                            title: "Playlists"
                            message: "Playlists are not indexed yet."
                        }
                    }

                    Component {
                        id: radioListComponent
                        PlaceholderList {
                            title: "Radio"
                            message: "Internet radio stations are not indexed yet."
                        }
                    }

                    Component {
                        id: albumEntryComponent
                        AlbumEntry {
                            onBack: contentStack.pop()
                        }
                    }
                }
            }
        }

        InferiorBar {
            id: player
            Layout.fillWidth: true
            Layout.preferredHeight: root.playH
        }
    }

    NowPlaying {
        id: nowPlaying
        anchors.fill: parent
        opacity: root.nowPlayingExpanded ? 1 : 0
        visible: opacity > 0

        // Same niri-like backdrop blur as the shell: the fullscreen player is
        // the backdrop when the search overlay opens from inside it.
        layer.enabled: root.visualMode || searchOverlay.opened
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: 0.5
            blurMax: 48
            saturation: 0.6
            autoPaddingEnabled: false
        }

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.emphasized; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    Toast {
        id: toast
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: root.playH + root.height * 0.02
        width: root.width * 0.42
        height: root.height * 0.06
        z: Theme.zToast
        onRetry: Backend.syncLibrary()
    }

    SearchOverlay {
        id: searchOverlay
        anchors.fill: parent

        // A search song hit goes through the same command pump as external
        // IPC, so the flow stays scriptable and single-sourced.
        onSongActivated: (id) => Backend.handleCommand("play_song_in_album " + id)
        onAlbumActivated: (albumId, title, artist, year, tracks, cover) => {
            var meta = [];
            if (year.length > 0)
                meta.push(year);
            if (tracks.length > 0)
                meta.push(tracks + " songs");
            contentStack.push(albumEntryComponent, {
                albumId: albumId,
                albumTitle: title,
                albumArtist: artist,
                albumCover: cover,
                albumMeta: meta.join("   ·   ")
            });
        }
        onArtistActivated: (artist) => contentStack.replace(albumListComponent, { artistFilter: artist })
    }

    QueueDrawer {
        id: queueDrawer
        anchors.fill: parent
    }

    Connections {
        target: Backend
        function onErrorRaised(kind, message) {
            toast.show(kind, message);
        }
        // Q-10 (roadmap D-4): activating a song opens Fullscreen Playback.
        // Album/artist activation keeps its navigation; the album-page IPC
        // flow below clears the flag again when it pushes the album page.
        function onPlaybackStarted() {
            nowPlayingExpanded = true;
        }
        // Single pumped event from `playSongInAlbum`: the queue + album
        // context. Deferred so the backend is not re-entered while the signal
        // is still being emitted.
        function onDataReady(channel, payload) {
            if (channel !== "song_album")
                return;
            var data = JSON.parse(payload);
            Qt.callLater(function() {
                Backend.playerLoad(data.queue, data.start);
                if (!data.album)
                    return;
                nowPlayingExpanded = false;
                var album = data.album;
                var meta = [];
                if (album.year)
                    meta.push(String(album.year));
                if (album.tracks)
                    meta.push(album.tracks + " songs");
                contentStack.push(albumEntryComponent, {
                    albumId: album.id,
                    albumTitle: album.title,
                    albumArtist: album.artist ? album.artist : "",
                    albumCover: album.cover ? album.cover : "",
                    albumMeta: meta.join("   ·   ")
                });
            });
        }
    }

    // Visual mode: niri-like backdrop blur (on the shell above) + dim, with the
    // focus border as the only chrome.
    Rectangle {
        anchors.fill: parent
        color: Palette.scrim
        opacity: root.visualMode && !root.nowPlayingExpanded ? Theme.scrimOpacity : 0
        z: Theme.zRegion - 1

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.standard; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    // Region borders = the vi cursor, positioned from the layout ratios.
    // 0 = sidebar, 1 = main, 2 = status (top), 3 = player (bottom).
    Repeater {
        model: [
            { color: Palette.primary, title: "LIBRARY", rotated: true },
            { color: Palette.tertiary, title: "MAIN", rotated: false },
            { color: Palette.secondary, title: "STATUS", rotated: false },
            { color: Palette.error, title: "PLAYER", rotated: false }
        ]

        delegate: Rectangle {
            id: regionBox
            required property int index
            required property var modelData

            readonly property bool active: index === root.activeRegion
            readonly property bool candidate: root.visualMode && index === root.visualSelection
            readonly property real unit: Math.max(1, root.width * 0.0015)

            x: index === 1 ? root.sideW : 0
            y: index === 2 ? 0 : (index === 3 ? root.height - root.playH : root.barH)
            width: (index === 2 || index === 3) ? root.width : (index === 1 ? root.width - root.sideW : root.sideW)
            height: index === 2 ? root.barH : (index === 3 ? root.playH : root.bodyH)
            z: Theme.zRegion
            color: "transparent"
            // Visual mode highlights only the candidate; otherwise only the
            // active (cursor) region is outlined. Fullscreen is an overlay, so
            // no region highlight may show above it.
            visible: !root.nowPlayingExpanded && (root.visualMode ? candidate : active)
            border.width: (candidate ? 3 : 2) * unit
            border.color: modelData.color

            // Width is instant (R5); only the colour eases.
            Behavior on border.color {
                ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            }

            // Region title shown only while choosing in visual mode.
            Text {
                anchors.centerIn: parent
                text: regionBox.modelData.title
                color: regionBox.modelData.color
                font.pixelSize: regionBox.modelData.rotated ? root.height * 0.04 : root.height * 0.03
                font.bold: true
                font.letterSpacing: root.height * 0.008
                rotation: regionBox.modelData.rotated ? -90 : 0
                opacity: root.visualMode ? 1 : 0
            }
        }
    }

    // Prime the status bar with the current counts once loaded.
    Timer {
        interval: 0
        running: true
        repeat: false
        onTriggered: Backend.refreshLibraryStatus()
    }
}
