import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Fullscreen player ("mainscreen"), a 50/50 split:
//   left   — cover, identity, seek and transport (everything about this track);
//   right  — Lyrics (top half) over Queue (bottom half), both always visible;
//   bottom — context-aware keybar for the focused pane (Tab cycles panes).
// Esc (or q) leaves fullscreen; Space stays the global play/pause.
Rectangle {
    id: root

    color: Theme.background

    // 0 = transport, 1 = lyrics, 2 = queue.
    property int pane: 0
    property int queueCursor: 0
    property int queueIndex: -1

    // Lyrics (override → cache → server; see src/lyrics.rs).
    property bool lyricsSynced: false
    property bool lyricsAvailable: false
    property string lyricsSource: ""
    property string lyricsLang: ""
    property int lyricsOffset: 0

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

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        var minutes = Math.floor(total / 60);
        var rest = total % 60;
        return minutes + ":" + (rest < 10 ? "0" : "") + rest;
    }

    function keyHints() {
        if (pane === 0)
            return "Space play/pause   ·   h/l seek   ·   n/p track   ·   Tab lyrics   ·   Esc exit";
        if (pane === 1)
            return "j/k scroll   ·   n/p track   ·   Tab queue   ·   Esc exit";
        return "j/k select   ·   Enter jump   ·   n/p track   ·   Tab transport   ·   Esc exit";
    }

    function scrollLyrics(delta) {
        var limit = Math.max(0, lyricsView.contentHeight - lyricsView.height);
        lyricsView.contentY = Math.max(0, Math.min(limit, lyricsView.contentY + delta));
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
                duration: formatTime(track.duration)
            });
        }
        queueIndex = data.index;
        queueCursor = Math.max(0, Math.min(data.index, Math.max(0, queueItems.count - 1)));
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

        lyricModel.clear();
        for (var i = 0; i < data.lines.length; i++) {
            lyricModel.append({
                t: data.lines[i].t === null || data.lines[i].t === undefined ? -1 : data.lines[i].t,
                text: data.lines[i].text
            });
        }
        lyricsAvailable = lyricModel.count > 0 && data.source !== "none";

        lyricsView.contentY = 0;
        if (lyricsSynced && activeLyric >= 0)
            lyricsView.positionViewAtIndex(activeLyric, ListView.Center);
    }

    function lyricsTag() {
        if (!lyricsAvailable)
            return "";
        var tag = lyricsSource.length > 0 ? lyricsSource : "";
        if (lyricsSynced)
            tag += " · synced";
        if (lyricsLang.length > 0)
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
                scrollLyrics(-lyricsFlick.height * 0.3);
            else if (action === "move_down")
                scrollLyrics(lyricsFlick.height * 0.3);
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
        else if (action === "enter") {
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
            if (root.visible)
                Backend.requestLyrics();
        }
    }

    ListModel {
        id: queueItems
    }

    ListModel {
        id: lyricModel
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
                text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "Nothing playing"
                color: Theme.onSurface
                font.pixelSize: Theme.fontSize("title", root.height)
                font.bold: true
                elide: Text.ElideRight
            }

            FadeText {
                text: "Esc exit full"
                color: Theme.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.height)
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

                Rectangle {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.maximumHeight: width
                    Layout.alignment: Qt.AlignHCenter
                    radius: Theme.radius
                    color: Theme.primaryContainer
                    clip: true

                    Image {
                        anchors.fill: parent
                        source: Backend.currentCoverUrl
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        cache: false
                        visible: status === Image.Ready
                    }

                    FadeText {
                        anchors.centerIn: parent
                        text: "MUSIKA"
                        color: Theme.onPrimaryContainer
                        font.pixelSize: Theme.fontSize("title", root.height)
                        font.letterSpacing: root.height * 0.006
                        visible: Backend.currentCoverUrl.length === 0
                    }
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist
                    color: Theme.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "No track playing"
                    color: Theme.onSurface
                    font.pixelSize: Theme.fontSize("headline", root.height)
                    font.bold: true
                    elide: Text.ElideRight
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Theme.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                // Seek row: elapsed, bar, duration.
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.width * 0.008

                    FadeText {
                        text: root.formatTime(Backend.position)
                        color: Theme.onSurfaceVariant
                        font.pixelSize: Theme.fontSize("mono", root.height)
                        animated: false
                    }

                    Rectangle {
                        id: seekTrack
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(2, root.height * 0.006)
                        color: Theme.outlineVariant
                        radius: Theme.radius

                        Rectangle {
                            width: seekTrack.width * (Backend.duration > 0 ? Math.min(1, Backend.position / Backend.duration) : 0)
                            height: parent.height
                            color: Theme.primary
                            radius: Theme.radius

                            Behavior on width {
                                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Linear }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            onPressed: (mouse) => {
                                if (Backend.duration > 0)
                                    Backend.playerSeek(Math.max(0, Math.min(1, mouse.x / width)) * Backend.duration);
                            }
                        }
                    }

                    FadeText {
                        text: root.formatTime(Backend.duration)
                        color: Theme.onSurfaceVariant
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
                    border.color: Theme.primary

                    TapHandler {
                        onTapped: root.pane = 0
                    }

                    RowLayout {
                        anchors.centerIn: parent
                        spacing: root.width * 0.012

                        IconButton {
                            Layout.preferredWidth: root.height * 0.05
                            Layout.preferredHeight: root.height * 0.05
                            symbol: "⏮"
                            symbolRatio: 0.6
                            onClicked: Backend.playerPrevious()
                        }
                        IconButton {
                            Layout.preferredWidth: root.height * 0.058
                            Layout.preferredHeight: root.height * 0.058
                            symbol: Backend.playbackState === "playing" ? "⏸" : "▶"
                            symbolRatio: 0.6
                            accent: true
                            onClicked: Backend.playerToggle()
                        }
                        IconButton {
                            Layout.preferredWidth: root.height * 0.05
                            Layout.preferredHeight: root.height * 0.05
                            symbol: "⏭"
                            symbolRatio: 0.6
                            onClicked: Backend.playerNext()
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
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    color: Theme.surfaceContainerLow
                    radius: Theme.radius
                    border.width: root.pane === 1 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Theme.primary

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
                                color: root.pane === 1 ? Theme.primary : Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item { Layout.fillWidth: true }

                            FadeText {
                                text: root.lyricsTag()
                                color: Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 1 ? Theme.primary : Theme.outlineVariant
                        }

                        ListView {
                            id: lyricsView
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            model: lyricModel
                            spacing: lyricsView.height * 0.018
                            boundsBehavior: Flickable.StopAtBounds
                            visible: root.lyricsAvailable

                            delegate: FadeText {
                                id: lyricLine
                                required property int index
                                required property real t
                                required property string text

                                width: lyricsView.width
                                text: index === root.activeLyric ? "> " + lyricLine.text + "  <" : lyricLine.text
                                color: index === root.activeLyric ? Theme.primary : Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("body", root.height)
                                font.bold: index === root.activeLyric
                                wrapMode: Text.WordWrap
                                animated: false

                                Behavior on color {
                                    ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: !root.lyricsAvailable
                            text: "No lyrics"
                            color: Theme.onSurfaceVariant
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
                    color: Theme.surfaceContainerLow
                    radius: Theme.radius
                    border.width: root.pane === 2 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Theme.primary
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
                                color: root.pane === 2 ? Theme.primary : Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item { Layout.fillWidth: true }

                            FadeText {
                                text: Backend.queueLength > 0 ? (Backend.queueIndex + 1) + " / " + Backend.queueLength : ""
                                color: Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 2 ? Theme.primary : Theme.outlineVariant
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
                                    ? Theme.secondaryContainer
                                    : (index === root.queueIndex ? Theme.surfaceContainer : "transparent")

                                FadeText {
                                    id: tag
                                    anchors.left: parent.left
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: queueList.width * 0.12
                                    text: index === root.queueIndex ? "Now" : (index < root.queueIndex ? "Prev" : "Next")
                                    color: index === root.queueIndex ? Theme.primary : Theme.onSurfaceVariant
                                    font.pixelSize: Theme.fontSize("label", root.height)
                                    font.bold: index === root.queueIndex
                                    animated: false
                                }

                                FadeText {
                                    anchors.left: tag.right
                                    anchors.right: durationLabel.left
                                    anchors.rightMargin: queueList.width * 0.01
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: row.title
                                    color: index === root.queueCursor ? Theme.onSecondaryContainer : Theme.onSurface
                                    font.pixelSize: Theme.fontSize("body", root.height)
                                    elide: Text.ElideRight
                                    animated: false
                                }

                                FadeText {
                                    id: durationLabel
                                    anchors.right: parent.right
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: row.duration
                                    color: Theme.onSurfaceVariant
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
                            color: Theme.onSurfaceVariant
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
            color: Theme.surfaceContainer
            radius: Theme.radius

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.leftMargin: parent.height * 0.4
                anchors.rightMargin: parent.height * 0.4
                text: root.keyHints()
                color: Theme.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.height)
                elide: Text.ElideRight
                animated: false
            }
        }
    }
}
