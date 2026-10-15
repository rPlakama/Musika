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
import QtQuick.Layouts
import QtQuick.Effects
import musika
import "ScrollMemory.js" as Memory
import "Theme.js" as Theme

// Fullscreen player ("mainscreen"), a 50/50 split:
//   left   — cover, identity, seek and transport (decoration: the transport
//            owns no keyboard pane, Space/n/p/,/. are global);
//   right  — Lyrics (top half) over Queue (bottom half), both always visible;
//   bottom — context-aware keybar for the focused pane (Tab cycles
//            lyrics/queue).
// Esc (or q) leaves fullscreen; Space stays the global play/pause.
Rectangle {
    id: root

    color: Palette.background

    /// Click/tap on the exit button (the header chip); Home leaves fullscreen.
    signal exitRequested
    /// ARC-UI-1: Alt-hold quick actions on the queue row under the cursor;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    // UI-57: the track title ramps with the window width like the list
    // headers do (ScrollPanel), toward the headline floor — never below it.
    readonly property real widthScale: Theme.widthScale(root.width)
    readonly property real trackTitlePixelSize: {
        var full = Theme.fontSize("headline", root.height);
        return Theme.clamp(full * root.widthScale, 18, full);
    }

    // Keyboard pane: 1 = lyrics, 2 = queue. Transport is never a pane.
    property int pane: 1
    property int queueCursor: 0
    property int queueIndex: -1
    // One queue payload after a reorder keeps the moved cursor instead of
    // snapping back to the playing track (Q-9).
    property bool _reordering: false
    // UI-54: first queue payload after entering restores the saved highlight.
    property bool _recallQueue: false

    // The active lyric glides to the pane centre; a manual scroll owns the
    // pane only until `lyricResume` fires (LYR-8): 5 s idle while lyrics is
    // the focused pane, 2 s when focus is elsewhere, so a stale line is never
    // left un-centred for long.
    property bool _lyricsHeld: false

    NumberAnimation {
        id: lyricFollow
        property: "contentY"
        duration: Theme.motion.standard
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.decelerate
    }

    // LYR-8: resume the centre-follow after the manual-scroll idle window.
    Timer {
        id: lyricResume
        repeat: false
        onTriggered: {
            root._lyricsHeld = false;
            root.followLyric();
        }
    }

    // Lyrics (override → cache → server; see src/lyrics.rs).
    property bool lyricsSynced: false
    property bool lyricsAvailable: false
    property bool lyricsLoading: false
    property string lyricsSource: ""
    property string lyricsLang: ""
    property int lyricsOffset: 0

    // Lyrics type is sized from the pane itself (parent rectangle), never the
    // window, so lines can't outgrow the pane's walls; the stage caps the
    // active emphasis to the padded room, so the grown line never reaches the
    // margin (see the delegate's `emphasis`).
    readonly property real lyricSize: Theme.clamp(lyricsPane.height * 0.055, 10, 18)
    readonly property real lyricPad: Math.max(6, lyricsPane.height * 0.04)

    // Active synced line, matched against the offset-corrected position.
    readonly property int activeLyric: {
        var count = lyricModel.count;
        if (!lyricsSynced || count === 0)
            return -1;
        var position = Math.round(Backend.position * 1000) - lyricsOffset;
        var active = -1;
        for (var i = 0; i < count; i++) {
            var start = lyricModel.get(i).t;
            if (start >= 0 && start <= position)
                active = i;
            else if (start > position)
                break;
        }
        return active;
    }

    function keyHints() {
        if (pane === 1)
            return "j/k scroll   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab queue   ·   Esc exit";
        return "j/k select   ·   Shift-J/K move   ·   Enter jump   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab lyrics   ·   Esc exit";
    }

    function scrollLyrics(delta) {
        root.holdLyrics();
        var limit = Math.max(0, lyricsView.contentHeight - lyricsView.height);
        lyricsView.contentY = Math.max(0, Math.min(limit, lyricsView.contentY + delta));
    }

    function holdLyrics() {
        root._lyricsHeld = true;
        lyricFollow.stop();
        lyricResume.interval = root.pane === 1 ? Theme.lyricsResume : Theme.lyricsResumeUnfocused;
        lyricResume.restart();
    }

    function followLyric() {
        if (root._lyricsHeld || !lyricsSynced || activeLyric < 0 || lyricModel.count === 0)
            return;
        if (Theme.reduceMotion) {
            lyricsView.positionViewAtIndex(activeLyric, ListView.Center);
            return;
        }
        var item = lyricsView.itemAtIndex(activeLyric);
        if (!item) {
            lyricsView.positionViewAtIndex(activeLyric, ListView.Center);
            return;
        }
        var limit = Math.max(0, lyricsView.contentHeight - lyricsView.height);
        var target = Math.max(0, Math.min(limit,
            item.y + item.height * 0.5 - lyricsView.height * 0.5));
        var distance = Math.abs(target - lyricsView.contentY);
        if (distance < 1)
            return;
        lyricFollow.stop();
        lyricFollow.target = lyricsView;
        lyricFollow.from = lyricsView.contentY;
        lyricFollow.to = target;
        lyricFollow.duration = distance < lyricsView.height * 0.3 ? Theme.motion.quick : Theme.motion.standard;
        lyricFollow.start();
    }

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }

    function moveQueueCursor(delta) {
        if (queueItems.count === 0)
            return;
        queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor + delta));
        // The view follows cursor changes itself (SCROLLCONTENTSTUDY.md).
        queueContent.currentIndex = queueCursor;
    }

    // VI-4: visual selection over the fullscreen queue. The state lives in
    // `queueContent` (the shared scroll model); Home routes `v`/`V`/Esc here
    // while the player is up.
    readonly property bool hasSelection: queueContent.hasSelection
    readonly property int selectionCount: queueContent.selectionCount

    function toggleSelection() {
        queueContent.toggleSelection();
    }

    function selectAll() {
        if (queueItems.count === 0)
            return;
        queueCursor = queueItems.count - 1;
        queueContent.currentIndex = queueCursor;
        queueContent.selectAllRows();
        queueContent.ensureVisible(queueCursor);
    }

    function clearSelection() {
        queueContent.clearSelection();
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`) — the
    /// fullscreen surface is its queue pane, so a jump focuses it and the
    /// context-Enter that follows lands on the matched row.
    function searchRows() {
        return queueContent.searchRows();
    }

    function searchJump(row) {
        pane = 2;
        queueContent.searchJump(row);
        queueCursor = queueContent.currentIndex;
    }

    function applyQueue(payload) {
        var data = JSON.parse(payload);
        // UI-41: payload swaps fade in quickly; a reorder keeps the list live.
        if (!root._reordering)
            queueContent.opacity = 0;
        queueItems.clear();
        for (var i = 0; i < data.items.length; i++) {
            var track = data.items[i];
            queueItems.append({
                title: track.title,
                duration: Theme.formatTime(track.duration),
                songId: track.id,
                albumId: track.album_id ? track.album_id : "",
                starred: track.starred === true
            });
        }
        queueIndex = data.index;
        if (root._reordering) {
            root._reordering = false;
            root._recallQueue = false;
            queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor));
            queueContent.currentIndex = queueCursor;
            queueContent.view.positionViewAtIndex(queueCursor, ListView.Contain);
        } else if (root._recallQueue && queueContent.restore()) {
            // UI-54: re-entry restores the queue highlight where it stopped.
            root._recallQueue = false;
            queueCursor = queueContent.currentIndex;
        } else {
            root._recallQueue = false;
            queueCursor = Math.max(0, Math.min(data.index, Math.max(0, queueItems.count - 1)));
            queueContent.currentIndex = queueCursor;
            queueContent.view.positionViewAtIndex(queueCursor, ListView.Contain);
        }
        Qt.callLater(function () {
            queueContent.opacity = 1;
        });
    }

    function applyLyrics(payload) {
        var data = JSON.parse(payload);
        if (data.song_id !== Backend.currentSongId)
            return; // stale response for a track we already left

        lyricsSource = data.source;
        lyricsSynced = data.synced;
        lyricsLang = data.lang ? data.lang : "";
        lyricsOffset = data.offset_ms ? data.offset_ms : 0;
        lyricsLoading = false;

        // Quick fade so a track change does not snap the lines (LYR-9).
        lyricsView.opacity = 0;
        lyricModel.clear();
        for (var i = 0; i < data.lines.length; i++) {
            lyricModel.append({
                t: data.lines[i].t === null || data.lines[i].t === undefined ? -1 : data.lines[i].t,
                line: data.lines[i].text
            });
        }
        lyricsAvailable = lyricModel.count > 0 && data.source !== "none";

        lyricFollow.stop();
        lyricResume.stop();
        lyricsView.contentY = 0;
        root._lyricsHeld = false;
        Qt.callLater(function () {
            lyricsView.opacity = 1;
            root.followLyric();
        });
    }

    function lyricsTag() {
        if (!lyricsAvailable)
            return "";
        var tag = lyricsSource.length > 0 ? lyricsSource : "";
        // `xxx`/`und` mean "unspecified" in OpenSubsonic; never show them.
        if (lyricsLang.length > 0 && lyricsLang !== "xxx" && lyricsLang !== "und")
            tag += " · " + lyricsLang;
        return tag;
    }

    onActiveLyricChanged: root.followLyric()

    // LYR-8: re-arm the idle window when pane focus moves (5 s vs 2 s).
    onPaneChanged: if (root._lyricsHeld) root.holdLyrics()

    /// ARC-UI-1: Alt armed the gesture — anchor on the queue row under the
    /// cursor; the fullscreen queue reuses the shared track verb set.
    function armQuickActions() {
        if (queueItems.count === 0 || queueContent.currentIndex < 0)
            return;
        var item = queueItems.get(queueContent.currentIndex);
        var row = queueContent.view.itemAtIndex(queueContent.currentIndex);
        var actions = root.hasSelection
            ? verbs.forSelection("song", queueContent.selectionIds("songId"))
            : (item ? verbs.song(item) : null);
        if (actions && row)
            quickAction(row, actions);
    }

    // Fullscreen keys, scoped to the focused pane. Esc/back stays with Home.
    function handleAction(action) {
        if (action === "pane_cycle") {
            pane = pane === 1 ? 2 : 1;
            return;
        }
        if (action === "next_track") {
            Backend.playerNext();
            return;
        }
        if (action === "prev_track") {
            Backend.playerPrevious();
            return;
        }

        if (pane === 1) {
            if (action === "move_up")
                scrollLyrics(-lyricsView.height * 0.3);
            else if (action === "move_down")
                scrollLyrics(lyricsView.height * 0.3);
            else if (action === "move_left" || action === "move_right")
                pane = 2;
            return;
        }

        if (action === "move_up")
            moveQueueCursor(-1);
        else if (action === "move_down")
            moveQueueCursor(1);
        else if (action === "queue_up" && queueCursor > 0) {
            root._reordering = true;
            // VI-4: a reorder shifts every anchored row; the range dies.
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor - 1);
            queueCursor -= 1;
            queueContent.currentIndex = queueCursor;
            return;
        } else if (action === "queue_down" && queueCursor < queueItems.count - 1) {
            root._reordering = true;
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor + 1);
            queueCursor += 1;
            queueContent.currentIndex = queueCursor;
            return;
        } else if (action === "enter") {
            // VI-4: jumping to a track ends the visual selection.
            queueContent.clearSelection();
            if (queueCursor >= 0)
                Backend.queueJump(queueCursor);
        } else if (action === "move_left" || action === "move_right")
            pane = 1;
    }

    onVisibleChanged: {
        if (visible) {
            // UI-54: re-entering restores the last fullscreen scope (pane) and
            // queue highlight; the first queue payload lands them.
            var saved = Memory.loadScroll("fullscreen");
            pane = saved && saved.scope >= 1 ? saved.scope : 1;
            root._recallQueue = true;
            lyricsLoading = true;
            Backend.requestQueue();
            Backend.requestLyrics();
        } else if (queueItems.count > 0) {
            queueContent.remember(pane);
            // VI-4: leaving fullscreen ends the queue selection.
            queueContent.clearSelection();
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "queue" && root.visible)
                root.applyQueue(payload);
            else if (channel === "lyrics" && root.visible)
                root.applyLyrics(payload);
        }
        function onCurrent_song_id_changed() {
            if (root.visible) {
                root.lyricsLoading = true;
                Backend.requestLyrics();
            }
        }
        function onTrackSkipped(forward) {
            if (forward)
                nextButton.press();
            else
                previousButton.press();
        }
        function onPlayToggled() {
            playButton.press();
        }
        // LYR-8: a seek (keys, seek bar, MPRIS) must not wait out the manual-
        // scroll resume window — drop the hold and re-centre at once, so the
        // lyrics land with the progress jump instead of 3/5 s later.
        function onSeeked() {
            root._lyricsHeld = false;
            lyricResume.stop();
            root.followLyric();
        }
    }

    ListModel {
        id: queueItems
    }

    // ARC-UI-1: the recycled Alt verb set; fullscreen queue rows are tracks.
    QuickActions {
        id: verbs
    }

    ListModel {
        id: lyricModel
    }

    // D-5 ambience: the playing cover through the shared `CoverImage.ambient`
    // mode (blurred and washed into the theme background, same rule as
    // AlbumEntry), so the square floats on its own colours without competing
    // with the lyrics/queue panes. Hidden without art; the panes stay opaque.
    // UI-55: one item for cover + wash, so the panes' acrylic samples exactly
    // what is painted behind them.
    Item {
        id: ambience
        anchors.fill: parent

        CoverImage {
            id: coverBackdrop
            anchors.fill: parent
            source: Backend.currentCoverUrl
            ambient: true
        }

        Rectangle {
            anchors.fill: parent
            visible: coverBackdrop.visible
            gradient: Gradient {
                GradientStop {
                    position: 0.0
                    color: root.withAlpha(Palette.background, 0.84)
                }
                GradientStop {
                    position: 0.38
                    color: root.withAlpha(Palette.background, 0.38)
                }
                GradientStop {
                    position: 1.0
                    color: root.withAlpha(Palette.background, 0.96)
                }
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.025
        spacing: root.height * 0.018

        // Header: track title + exit hint.
        RowLayout {
            Layout.fillWidth: true

            FadeText {
                Layout.fillWidth: true
                text: "MUSIKA"
                color: Palette.onSurface
                font.family: Theme.fonts.wordmark
                font.pixelSize: Theme.fontSize("title", root.height)
                font.bold: true
                elide: Text.ElideRight
            }

            // Exit chip: same shared control as the album/artist headers.
            ExitChip {
                host: root
                label: "ESC/Q"
                padRatio: 0.02
                onClicked: root.exitRequested()
            }
        }

        // Body: left (cover + transport) and right (lyrics over queue), 50/50.
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: root.width * 0.02

            // ---- Left column: cover, identity, seek, transport ----
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: root.width * 0.48
                spacing: root.height * 0.012

                // D-3 cover rule, shared with AlbumEntry: a square sized by
                // the available box and the source ceiling (512), centered in
                // the flexible area. Width never stretches the container, so
                // no window shape crops the art.
                Item {
                    id: coverArea
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    Rectangle {
                        id: coverBox
                        readonly property real side: Math.max(96, Math.min(root.height * 0.42, 512, parent.width, parent.height))
                        anchors.centerIn: parent
                        width: side
                        height: side
                        radius: Theme.radius
                        color: Palette.primaryContainer
                        clip: true

                        CoverImage {
                            anchors.fill: parent
                            source: Backend.currentCoverUrl
                        }

                        FadeText {
                            anchors.centerIn: parent
                            text: "MUSIKA"
                            color: Palette.onPrimaryContainer
                            font.family: Theme.fonts.wordmark
                            font.pixelSize: Theme.fontSize("title", root.height)
                            font.letterSpacing: root.height * 0.006
                            visible: Backend.currentCoverUrl.length === 0
                        }
                    }
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "No track playing"
                    color: Palette.onSurface
                    font.pixelSize: root.trackTitlePixelSize
                    font.bold: true
                    elide: Text.ElideRight
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                // Seek row: elapsed, bar, duration.
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.width * 0.008

                    FadeText {
                        text: Theme.formatTime(Backend.position)
                        color: Palette.onSurfaceVariant
                        font.pixelSize: Theme.fontSize("mono", root.height)
                        animated: false
                    }

                    Rectangle {
                        id: seekTrack
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(2, root.height * 0.006)
                        color: Palette.outlineVariant
                        radius: Theme.radius

                        Rectangle {
                            width: seekTrack.width * (Backend.duration > 0 ? Math.min(1, Backend.position / Backend.duration) : 0)
                            height: parent.height
                            color: Palette.primary
                            radius: Theme.radius

                            Behavior on width {
                                NumberAnimation {
                                    duration: Theme.motion.quick
                                    easing.type: Easing.Linear
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onPressed: mouse => {
                                if (Backend.duration > 0)
                                    Backend.playerSeek(Math.max(0, Math.min(1, mouse.x / width)) * Backend.duration);
                            }
                        }
                    }

                    FadeText {
                        text: Theme.formatTime(Backend.duration)
                        color: Palette.onSurfaceVariant
                        font.pixelSize: Theme.fontSize("mono", root.height)
                        animated: false
                    }
                }

                // Transport bar: mouse-only decoration (UI-51; no focus
                // hairline, Space/n/p/,/. are global). One major rectangle
                // owns both clusters as their own rectangles: the volume box
                // (right) yields its bar width first, and the transport box
                // centres in the room left of it, sliding left as the column
                // narrows — the two can never collide.
                Rectangle {
                    id: transportBar
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.height * 0.07
                    color: "transparent"
                    radius: Theme.radius

                    // Volume cluster (P-7): tiered icon (click = mute) + bar.
                    // The bar is the elastic part; the box clamps to whatever
                    // the transport is not using.
                    Rectangle {
                        id: volumeBox
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.max(root.height * 0.04,
                            Math.min(volumeRow.implicitWidth,
                                transportBar.width - transportBox.width - root.width * 0.012))
                        height: parent.height
                        color: "transparent"

                        RowLayout {
                            id: volumeRow
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width
                            spacing: root.width * 0.008

                            IconButton {
                                Layout.preferredWidth: root.height * 0.04
                                Layout.preferredHeight: root.height * 0.04
                                iconSource: Theme.volumeIcon(Backend.volume, Backend.muted)
                                symbolRatio: 0.95
                                onClicked: Backend.playerToggleMute()
                            }

                            Rectangle {
                                id: volumeTrack
                                Layout.fillWidth: true
                                Layout.minimumWidth: root.height * 0.02
                                Layout.maximumWidth: root.width * 0.06
                                Layout.preferredWidth: root.width * 0.06
                                Layout.preferredHeight: Math.max(2, root.height * 0.006)
                                color: Palette.outlineVariant
                                radius: Theme.radius

                                Rectangle {
                                    width: parent.width * Math.max(0, Math.min(1, Backend.volume / 100))
                                    height: parent.height
                                    radius: Theme.radius
                                    color: Backend.muted ? Palette.outline : Palette.primary

                                    Behavior on width {
                                        NumberAnimation {
                                            duration: Theme.motion.quick
                                            easing.type: Easing.Linear
                                        }
                                    }
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onPressed: mouse => Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100)
                                    onPositionChanged: mouse => {
                                        if (pressed)
                                            Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100);
                                    }
                                }
                            }
                        }
                    }

                    // Transport cluster (skip / pause / next) in its own
                    // rectangle: centred in the whole bar while there is room,
                    // and pushed left only as far as needed once the volume
                    // box would touch it — the transport yields, the volume
                    // stays put.
                    Rectangle {
                        id: transportBox
                        anchors.verticalCenter: parent.verticalCenter
                        width: transportRow.implicitWidth
                        height: parent.height
                        color: "transparent"
                        x: Math.max(0, Math.min(
                            (transportBar.width - width) / 2,
                            transportBar.width - volumeBox.width
                                - root.width * 0.012 - width))

                        RowLayout {
                            id: transportRow
                            anchors.centerIn: parent
                            spacing: root.width * 0.012

                            IconButton {
                                id: previousButton
                                Layout.preferredWidth: root.height * 0.05
                                Layout.preferredHeight: root.height * 0.05
                                iconSource: "qrc:/app/icons/skip-previous-solid-svgrepo-com.svg"
                                symbolRatio: 0.9
                                onClicked: Backend.playerPrevious()
                            }
                            IconButton {
                                id: playButton
                                Layout.preferredWidth: root.height * 0.058
                                Layout.preferredHeight: root.height * 0.058
                                symbolRatio: 0.6
                                iconSource: Backend.playbackState === "playing" ? "qrc:/app/icons/pause-svgrepo-com.svg" : "qrc:/app/icons/play-1001-svgrepo-com.svg"
                                symbolColor: Theme.iconStrong
                                onClicked: Backend.playerToggle()
                            }
                            IconButton {
                                id: nextButton
                                Layout.preferredWidth: root.height * 0.05
                                Layout.preferredHeight: root.height * 0.05
                                iconSource: "qrc:/app/icons/skip-next-circle-solid-svgrepo-com.svg"
                                symbolRatio: 0.9
                                onClicked: Backend.playerNext()
                            }
                        }
                    }
                }
            }

            // ---- Right column: Lyrics (top) over Queue (bottom) ----
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: root.width * 0.48
                spacing: root.height * 0.012

                // Lyrics, top half. UI-55: the solid pane over the ambient
                // blur becomes acrylic (blurred backdrop + tint + grain).
                AcrylicPanel {
                    id: lyricsPane
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    tint: Palette.surfaceContainerLow
                    backdrop: coverBackdrop.visible ? ambience : null
                    radius: Theme.radius
                    border.width: root.pane === 1 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Palette.primary

                    TapHandler {
                        onTapped: root.pane = 1
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: root.width * 0.012
                        spacing: root.height * 0.006

                        RowLayout {
                            Layout.fillWidth: true

                            FadeText {
                                text: "Lyrics"
                                color: root.pane === 1 ? Palette.primary : Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            FadeText {
                                text: root.lyricsTag()
                                color: Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }

                            // Synced as a small MY3 chip in `label` type.
                            Rectangle {
                                visible: root.lyricsAvailable && root.lyricsSynced
                                implicitWidth: syncedLabel.implicitWidth + root.height * 0.02
                                implicitHeight: root.height * 0.026
                                radius: Theme.radius
                                color: Palette.secondaryContainer

                                FadeText {
                                    id: syncedLabel
                                    anchors.centerIn: parent
                                    text: "SYNCED"
                                    color: Palette.onSecondaryContainer
                                    font.pixelSize: Theme.fontSize("label", root.height)
                                    font.letterSpacing: root.height * 0.001
                                    animated: false
                                }
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 1 ? Palette.primary : Palette.outlineVariant
                        }

                        // The stage: an inner rectangle inside the pane that
                        // contains every lyric line, the scaled active one
                        // included. Delegates pre-calculate their emphasis so
                        // the grown line never reaches these walls; `clip` is
                        // the last-resort containment.
                        Rectangle {
                            id: lyricsStage
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            color: "transparent"
                            clip: true

                            ListView {
                                id: lyricsView
                                anchors.fill: parent
                                model: lyricModel
                                spacing: root.lyricSize * 0.9
                                boundsBehavior: Flickable.StopAtBounds
                                visible: root.lyricsAvailable

                                // A hand on the pane owns it until the next
                                // track.
                                onMovementStarted: root.holdLyrics()

                                Behavior on opacity {
                                    NumberAnimation {
                                        duration: Theme.motion.quick
                                        easing.type: Easing.Bezier
                                        easing.bezierCurve: Theme.ease.standard
                                    }
                                }

                                delegate: FadeText {
                                    id: lyricLine
                                    required property int index
                                    required property real t
                                    required property string line

                                    // Lyrics stage (roadmap "Lyrics presentation"):
                                    // active = scale + full contrast, near = dim,
                                    // far = dimmer; unsynced is one uniform column.
                                    readonly property bool active: index === root.activeLyric
                                    readonly property int distance: root.activeLyric >= 0 ? Math.abs(index - root.activeLyric) : -1

                                    // Pre-calculated emphasis: the full 1.12
                                    // while the wrapped line stays clear of the
                                    // margin, otherwise exactly the scale that
                                    // fits between the paddings (never touching
                                    // the stage walls). Long unbreakable words
                                    // can shrink below 1 instead of being cut.
                                    readonly property real emphasis: {
                                        if (!lyricLine.active)
                                            return 1.0;
                                        var room = lyricLine.width - lyricLine.leftPadding - lyricLine.rightPadding;
                                        if (room <= 0 || lyricLine.paintedWidth <= 0)
                                            return Theme.lyricsEmphasis;
                                        return Math.min(Theme.lyricsEmphasis, room / lyricLine.paintedWidth);
                                    }
                                    // Grow the slot symmetrically so the big
                                    // line stays centred and its neighbours
                                    // make room instead of getting overlapped.
                                    readonly property real emphasisPad: Math.max(0, lyricLine.paintedHeight * (lyricLine.emphasis - 1) / 2)

                                    width: lyricsView.width
                                    leftPadding: root.lyricPad
                                    rightPadding: root.lyricPad
                                    topPadding: lyricLine.emphasisPad
                                    bottomPadding: lyricLine.emphasisPad
                                    text: lyricLine.line
                                    horizontalAlignment: Text.AlignHCenter
                                    color: lyricLine.active ? Palette.onSurface : Palette.onSurfaceVariant
                                    font.family: Theme.fonts.lyrics
                                    font.pixelSize: root.lyricSize
                                    wrapMode: Text.WordWrap
                                    animated: false
                                    transformOrigin: Item.Center
                                    opacity: lyricLine.distance < 0 ? Theme.contentDim : (lyricLine.active ? 1.0 : (lyricLine.distance === 1 ? Theme.contentDim : Theme.contentDimFar))
                                    scale: lyricLine.emphasis

                                    Behavior on opacity {
                                        NumberAnimation {
                                            duration: Theme.motion.quick
                                            easing.type: Easing.Bezier
                                            easing.bezierCurve: Theme.ease.standard
                                        }
                                    }
                                    Behavior on scale {
                                        NumberAnimation {
                                            duration: Theme.motion.quick
                                            easing.type: Easing.Bezier
                                            easing.bezierCurve: Theme.ease.standard
                                        }
                                    }
                                    Behavior on color {
                                        ColorAnimation {
                                            duration: Theme.motion.micro
                                            easing.type: Easing.Bezier
                                            easing.bezierCurve: Theme.ease.standard
                                        }
                                    }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: root.lyricsLoading
                            text: "Loading lyrics…"
                            color: Palette.onSurfaceVariant
                            font.family: Theme.fonts.lyrics
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: !root.lyricsAvailable && !root.lyricsLoading
                            text: "No lyrics"
                            color: Palette.onSurfaceVariant
                            font.family: Theme.fonts.lyrics
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }

                // Queue, bottom half. UI-55: acrylic like the lyrics pane.
                AcrylicPanel {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    tint: Palette.surfaceContainerLow
                    backdrop: coverBackdrop.visible ? ambience : null
                    radius: Theme.radius
                    border.width: root.pane === 2 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Palette.primary
                    clip: true

                    TapHandler {
                        onTapped: root.pane = 2
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: root.width * 0.012
                        spacing: root.height * 0.006

                        RowLayout {
                            Layout.fillWidth: true

                            FadeText {
                                text: "Queue"
                                color: root.pane === 2 ? Palette.primary : Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            // VI-4: the selection count while a range is up.
                            SelectionBadge {
                                Layout.alignment: Qt.AlignVCenter
                                count: root.selectionCount
                                pixelSize: Theme.fontSize("label", root.height)
                            }

                            FadeText {
                                text: Backend.queueLength > 0 ? (Backend.queueIndex + 1) + " / " + Backend.queueLength : ""
                                color: Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 2 ? Palette.primary : Palette.outlineVariant
                        }

                        Item {
                            Layout.fillWidth: true
                            Layout.fillHeight: true

                            // UI-48: the shared scroll body carries the
                            // overflow stage — the highlight bleeds into the
                            // gutters while rows still clip vertically at the
                            // list bounds.
                            ScrollContent {
                                id: queueContent
                                anchors.fill: parent
                                bleed: width * 0.012
                                model: queueItems
                                selectable: true
                                memoryKey: "fullscreen"
                                // ARC-UI-2 M1: the queue rows paint title
                                // and duration.
                                searchFields: ["title", "duration"]
                                // The fullscreen memory also stores the focused
                                // pane; without this the continuous remembers
                                // clobbered it to the default scope.
                                memoryScope: root.pane
                                visible: queueItems.count > 0

                                Behavior on opacity {
                                    NumberAnimation {
                                        duration: Theme.motion.quick
                                        easing.type: Easing.Bezier
                                        easing.bezierCurve: Theme.ease.standard
                                    }
                                }

                                rowDelegate: Component {
                                    Rectangle {
                                        id: row
                                        required property int index
                                        required property string title
                                        required property string duration
                                        required property bool starred

                                        readonly property bool emphasized: index === root.queueCursor || rowHover.hovered

                                        width: ListView.view.width
                                        height: ListView.view.height * 0.16
                                        // Playing-track tint stays on the row; the
                                        // cursor/hover block is the shared UI-48 layer.
                                        color: index === root.queueIndex ? Palette.surfaceContainer : "transparent"

                                        RowHighlight {
                                            current: index === root.queueCursor
                                            hovered: rowHover.hovered
                                            selected: queueContent.selected(index)
                                            overflow: queueContent.bleed
                                        }

                                        FadeText {
                                            id: titleText
                                            anchors.left: parent.left
                                            anchors.leftMargin: ListView.view.width * 0.01
                                            anchors.right: durationLabel.left
                                            anchors.rightMargin: ListView.view.width * 0.01
                                            anchors.verticalCenter: parent.verticalCenter
                                            text: row.title
                                            color: index === root.queueCursor ? Palette.onSecondaryContainer : Palette.onSurface
                                            font.pixelSize: Theme.fontSize("body", root.height)
                                            elide: Text.ElideRight
                                            animated: false
                                            transformOrigin: Item.Left
                                            // UI-45: the cursor/hover row leads, capped to its box.
                                            scale: (row.emphasized && !Theme.reduceMotion) ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis) : 1.0

                                            Behavior on scale {
                                                NumberAnimation {
                                                    duration: Theme.motion.quick
                                                    easing.type: Easing.Bezier
                                                    easing.bezierCurve: Theme.ease.standard
                                                }
                                            }
                                        }

                                        FadeText {
                                            id: durationLabel
                                            anchors.right: parent.right
                                            anchors.verticalCenter: parent.verticalCenter
                                            text: row.duration
                                            color: Palette.onSurfaceVariant
                                            font.pixelSize: Theme.fontSize("mono", root.height)
                                            animated: false
                                        }

                                        // UI-56: the favourite heart leads the
                                        // duration (state mark, no hitbox).
                                        Icon {
                                            visible: row.starred
                                            anchors.verticalCenter: parent.verticalCenter
                                            anchors.right: durationLabel.left
                                            anchors.rightMargin: ListView.view.width * 0.01
                                            width: durationLabel.font.pixelSize * 0.8
                                            height: width
                                            name: "baseline-favorite"
                                            symbolColor: Theme.iconStrong
                                        }

                                        HoverHandler {
                                            id: rowHover
                                            cursorShape: Qt.PointingHandCursor
                                        }

                                        TapHandler {
                                            onTapped: {
                                                root.queueCursor = row.index;
                                                Backend.queueJump(row.index);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: queueItems.count === 0
                            text: "Queue is empty"
                            color: Palette.onSurfaceVariant
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }
            }
        }

        // Context-aware keybar.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: root.height * 0.055
            color: Palette.surfaceContainer
            radius: Theme.radius

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: parent.height * 0.4
                anchors.rightMargin: parent.height * 0.4
                text: root.keyHints()
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.height)
                elide: Text.ElideRight
                animated: false
            }
        }
    }
}
