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

    // TODO(M6): real lyrics with sync; the marked line is a placeholder stub.
    property int currentLyric: 4
    readonly property var lyricLines: [
        "In the deepest ocean",
        "The bottom of the sea",
        "Your eyes",
        "They turn me",
        "Why should I stay here?",
        "Why should I stay?",
        "I'd be crazy not to follow",
        "Follow where you lead",
        "Your eyes",
        "They turn me",
        "Turn me on to phantoms",
        "I follow to the edge of the earth",
        "And fall off",
        "Everybody leaves",
        "If they get the chance",
        "And this is my chance"
    ]

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
        var maxY = Math.max(0, lyricsFlick.contentHeight - lyricsFlick.height);
        lyricsFlick.contentY = Math.max(0, Math.min(maxY, lyricsFlick.contentY + delta));
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
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "queue" && root.visible)
                root.applyQueue(payload);
        }
    }

    ListModel {
        id: queueItems
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
                                text: "placeholder · M6"
                                color: Theme.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 1 ? Theme.primary : Theme.outlineVariant
                        }

                        Flickable {
                            id: lyricsFlick
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            clip: true
                            contentWidth: width
                            contentHeight: lyricsColumn.height
                            boundsBehavior: Flickable.StopAtBounds

                            Column {
                                id: lyricsColumn
                                width: lyricsFlick.width
                                spacing: lyricsFlick.height * 0.018

                                Repeater {
                                    model: root.lyricLines

                                    delegate: FadeText {
                                        required property int index
                                        required property string modelData

                                        width: lyricsColumn.width
                                        text: index === root.currentLyric ? "> " + modelData + "  <" : modelData
                                        color: index === root.currentLyric ? Theme.primary : Theme.onSurfaceVariant
                                        font.pixelSize: Theme.fontSize("body", root.height)
                                        font.bold: index === root.currentLyric
                                        wrapMode: Text.WordWrap
                                        animated: false
                                    }
                                }
                            }
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
