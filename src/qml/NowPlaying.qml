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
import "Theme.js" as Theme

// Fullscreen player ("mainscreen"), a 50/50 split:
//   left   — cover, identity, seek and transport (everything about this track);
//   right  — Lyrics (top half) over Queue (bottom half), both always visible;
//   bottom — context-aware keybar for the focused pane (Tab cycles panes).
// Esc (or q) leaves fullscreen; Space stays the global play/pause.
Rectangle {
    id: root

    color: Palette.background

    /// Click/tap on the exit button (the header chip); Home leaves fullscreen.
    signal exitRequested()

    // 0 = transport, 1 = lyrics, 2 = queue.
    property int pane: 0
    property int queueCursor: 0
    property int queueIndex: -1
    // One queue payload after a reorder keeps the moved cursor instead of
    // snapping back to the playing track (Q-9).
    property bool _reordering: false

    // Lyrics (override → cache → server; see src/lyrics.rs).
    property bool lyricsSynced: false
    property bool lyricsAvailable: false
    property bool lyricsLoading: false
    property string lyricsSource: ""
    property string lyricsLang: ""
    property int lyricsOffset: 0

    // Lyrics type is sized from the pane itself (parent rectangle), never the
    // window, so lines can't outgrow the pane's walls; the padding keeps the
    // scaled active line off the edges.
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
        if (pane === 0)
            return "Space play/pause   ·   h/l seek   ·   Ctrl-N/P track   ·   m mute   ·   -/+ volume   ·   Tab lyrics   ·   Esc exit";
        if (pane === 1)
            return "j/k scroll   ·   Ctrl-N/P track   ·   Tab queue   ·   Esc exit";
        return "j/k select   ·   Alt-J/K move   ·   Enter jump   ·   Ctrl-N/P track   ·   Tab transport   ·   Esc exit";
    }

    function scrollLyrics(delta) {
        var limit = Math.max(0, lyricsView.contentHeight - lyricsView.height);
        lyricsView.contentY = Math.max(0, Math.min(limit, lyricsView.contentY + delta));
    }

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }

    function moveQueueCursor(delta) {
        if (queueItems.count === 0)
            return;
        queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor + delta));
        queueList.currentIndex = queueCursor;
        queueList.positionViewAtIndex(queueCursor, ListView.Contain);
    }

    function applyQueue(payload) {
        var data = JSON.parse(payload);
        queueItems.clear();
        for (var i = 0; i < data.items.length; i++) {
            var track = data.items[i];
            queueItems.append({
                title: track.title,
                duration: Theme.formatTime(track.duration)
            });
        }
        queueIndex = data.index;
        if (root._reordering) {
            root._reordering = false;
            queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor));
        } else {
            queueCursor = Math.max(0, Math.min(data.index, Math.max(0, queueItems.count - 1)));
        }
        queueList.currentIndex = queueCursor;
        queueList.positionViewAtIndex(queueCursor, ListView.Contain);
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

        lyricsView.contentY = 0;
        Qt.callLater(function() { lyricsView.opacity = 1; });
        if (lyricsSynced && activeLyric >= 0)
            lyricsView.positionViewAtIndex(activeLyric, ListView.Center);
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

    onActiveLyricChanged: {
        if (lyricsSynced && activeLyric >= 0 && lyricsView.count > 0)
            lyricsView.positionViewAtIndex(activeLyric, ListView.Center);
    }

    // Fullscreen keys, scoped to the focused pane. Esc/back stays with Home.
    function handleAction(action) {
        if (action === "pane_cycle") {
            pane = (pane + 1) % 3;
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

        if (pane === 0) {
            if (action === "move_left")
                Backend.playerSeek(Math.max(0, Backend.position - 5));
            else if (action === "move_right")
                Backend.playerSeek(Backend.position + 5);
            else if (action === "move_up")
                Backend.playerPrevious();
            else if (action === "move_down")
                Backend.playerNext();
            else if (action === "enter")
                Backend.playerToggle();
            return;
        }

        if (pane === 1) {
            if (action === "move_up")
                scrollLyrics(-lyricsView.height * 0.3);
            else if (action === "move_down")
                scrollLyrics(lyricsView.height * 0.3);
            else if (action === "move_left")
                pane = 0;
            else if (action === "move_right")
                pane = 2;
            return;
        }

        if (action === "move_up")
            moveQueueCursor(-1);
        else if (action === "move_down")
            moveQueueCursor(1);
        else if (action === "queue_up" && queueCursor > 0) {
            root._reordering = true;
            Backend.queueMove(queueCursor, queueCursor - 1);
            queueCursor -= 1;
            queueList.currentIndex = queueCursor;
            queueList.positionViewAtIndex(queueCursor, ListView.Contain);
            return;
        } else if (action === "queue_down" && queueCursor < queueItems.count - 1) {
            root._reordering = true;
            Backend.queueMove(queueCursor, queueCursor + 1);
            queueCursor += 1;
            queueList.currentIndex = queueCursor;
            queueList.positionViewAtIndex(queueCursor, ListView.Contain);
            return;
        } else if (action === "enter") {
            if (queueCursor >= 0)
                Backend.queueJump(queueCursor);
        } else if (action === "move_left")
            pane = 0;
        else if (action === "move_right")
            pane = 1;
    }

    onVisibleChanged: {
        if (visible) {
            pane = 0;
            lyricsLoading = true;
            Backend.requestQueue();
            Backend.requestLyrics();
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
    }

    ListModel {
        id: queueItems
    }

    ListModel {
        id: lyricModel
    }

    // D-5 ambience: the playing cover, blurred and washed into the theme
    // background (same rule as AlbumEntry), so the square floats on its own
    // colours without competing with the lyrics/queue panes. Hidden without
    // art; the panes stay opaque on top.
    Image {
        id: coverBackdrop
        anchors.fill: parent
        source: Backend.currentCoverUrl
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        cache: false
        opacity: (status === Image.Ready && Backend.currentCoverUrl.length > 0) ? 1 : 0
        visible: opacity > 0

        layer.enabled: visible
        layer.effect: MultiEffect {
            blurEnabled: true
            blur: 1.0
            blurMax: 64
            saturation: 0.6
            autoPaddingEnabled: false
        }

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }
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
                font.pixelSize: Theme.fontSize("title", root.height)
                font.bold: true
                elide: Text.ElideRight
            }

            // Exit chip, same style as the AlbumEntry back button: exit icon +
            // "ESC" inside the tonal rectangle (D-6 SymbolIcon, strong glyph).
            Rectangle {
                id: exitButton
                implicitWidth: exitContent.implicitWidth + root.width * 0.02
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: exitHover.hovered ? Palette.secondaryContainer : Palette.surfaceContainerHigh
                border.color: Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                Row {
                    id: exitContent
                    anchors.centerIn: parent
                    spacing: root.height * 0.01

                    Item {
                        anchors.verticalCenter: parent.verticalCenter
                        width: root.height * 0.026
                        height: width

                        SymbolIcon {
                            anchors.centerIn: parent
                            source: "qrc:/app/icons/exit-2-svgrepo-com.svg"
                            symbolColor: Theme.iconStrong
                            width: parent.width * 0.85
                            height: width
                        }
                    }

                    FadeText {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "ESC"
                        color: Palette.onSurface
                        font.pixelSize: root.height * 0.022
                    }
                }

                HoverHandler {
                    id: exitHover
                    cursorShape: Qt.PointingHandCursor
                }
                TapHandler {
                    onTapped: root.exitRequested()
                }
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
                        readonly property real side: Math.max(
                            96,
                            Math.min(root.height * 0.42, 512, parent.width, parent.height))
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
                    font.pixelSize: Theme.fontSize("headline", root.height)
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
                                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Linear }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onPressed: (mouse) => {
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

                // Transport row; a hairline marks keyboard focus on this pane.
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.height * 0.07
                    color: "transparent"
                    radius: Theme.radius
                    border.width: root.pane === 0 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Palette.primary

                    TapHandler {
                        onTapped: root.pane = 0
                    }

                    RowLayout {
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
                            Layout.preferredWidth: root.height * 0.058
                            Layout.preferredHeight: root.height * 0.058
                            symbolRatio: 0.6
                            iconSource: Backend.playbackState === "playing"
                                ? "qrc:/app/icons/pause-svgrepo-com.svg"
                                : "qrc:/app/icons/play-1001-svgrepo-com.svg"
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

                    // Volume cluster (P-7), right-anchored so play/pause stays
                    // centred: tiered icon (click = mute) + compact bar.
                    RowLayout {
                        anchors.right: parent.right
                        anchors.rightMargin: root.width * 0.012
                        anchors.verticalCenter: parent.verticalCenter
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
                                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Linear }
                                }
                            }

                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onPressed: (mouse) => Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100)
                                onPositionChanged: (mouse) => {
                                    if (pressed)
                                        Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100);
                                }
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

                // Lyrics, top half.
                Rectangle {
                    id: lyricsPane
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Palette.surfaceContainerLow
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

                            Item { Layout.fillWidth: true }

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

                        ListView {
                            id: lyricsView
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: lyricModel
                            spacing: root.lyricSize * 0.9
                            boundsBehavior: Flickable.StopAtBounds
                            visible: root.lyricsAvailable

                            Behavior on opacity {
                                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
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
                                readonly property int distance: root.activeLyric >= 0
                                    ? Math.abs(index - root.activeLyric) : -1

                                width: lyricsView.width
                                leftPadding: root.lyricPad
                                rightPadding: root.lyricPad
                                text: lyricLine.line
                                horizontalAlignment: Text.AlignHCenter
                                color: lyricLine.active ? Palette.onSurface : Palette.onSurfaceVariant
                                font.pixelSize: root.lyricSize
                                wrapMode: Text.WordWrap
                                animated: false
                                transformOrigin: Item.Center
                                opacity: lyricLine.distance < 0
                                    ? Theme.contentDim
                                    : (lyricLine.active
                                        ? 1.0
                                        : (lyricLine.distance === 1 ? Theme.contentDim : Theme.contentDimFar))
                                scale: lyricLine.active ? Theme.lyricsEmphasis : 1.0

                                Behavior on opacity {
                                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                                Behavior on scale {
                                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                                Behavior on color {
                                    ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: root.lyricsLoading
                            text: "Loading lyrics…"
                            color: Palette.onSurfaceVariant
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
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }

                // Queue, bottom half.
                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Palette.surfaceContainerLow
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

                            Item { Layout.fillWidth: true }

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

                        ListView {
                            id: queueList
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: queueItems
                            boundsBehavior: Flickable.StopAtBounds
                            visible: queueItems.count > 0

                            delegate: Rectangle {
                                id: row
                                required property int index
                                required property string title
                                required property string duration

                                width: queueList.width
                                height: queueList.height * 0.16
                                color: index === root.queueCursor
                                    ? Palette.secondaryContainer
                                    : (index === root.queueIndex ? Palette.surfaceContainer : "transparent")

                                FadeText {
                                    anchors.left: parent.left
                                    anchors.leftMargin: queueList.width * 0.01
                                    anchors.right: durationLabel.left
                                    anchors.rightMargin: queueList.width * 0.01
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: row.title
                                    color: index === root.queueCursor ? Palette.onSecondaryContainer : Palette.onSurface
                                    font.pixelSize: Theme.fontSize("body", root.height)
                                    elide: Text.ElideRight
                                    animated: false
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

                                TapHandler {
                                    onTapped: {
                                        root.queueCursor = row.index;
                                        Backend.queueJump(row.index);
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
