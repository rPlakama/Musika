import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Inferior bar: the playback controls, driven by the ffplay engine.
Rectangle {
    id: root

    color: Theme.surfaceContainer

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        var minutes = Math.floor(total / 60);
        var rest = total % 60;
        return minutes + ":" + (rest < 10 ? "0" : "") + rest;
    }

    // Polls the player for position and end-of-track.
    Timer {
        interval: 500
        running: true
        repeat: true
        onTriggered: Backend.tick()
    }

    // Seek line across the top edge.
    Rectangle {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: Math.max(1, root.height * 0.03)
        color: Theme.outlineVariant

        Rectangle {
            width: parent.width * (Backend.duration > 0
                ? Math.min(1, Backend.position / Backend.duration)
                : 0)
            height: parent.height
            color: Theme.primary
        }
    }

    // Click/drag anywhere on the top strip to seek.
    MouseArea {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: Math.max(root.height * 0.14, 8)
        enabled: Backend.duration > 0

        function seekTo(x) {
            if (Backend.duration <= 0)
                return;
            var fraction = Math.max(0, Math.min(1, x / width));
            Backend.playerSeek(fraction * Backend.duration);
        }

        onPressed: (mouse) => seekTo(mouse.x)
        onPositionChanged: (mouse) => {
            if (pressed)
                seekTo(mouse.x);
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.013
        anchors.rightMargin: root.width * 0.013
        spacing: root.width * 0.012

        // Current track.
        RowLayout {
            spacing: root.height * 0.12

            Rectangle {
                Layout.preferredWidth: root.height * 0.62
                Layout.preferredHeight: root.height * 0.62
                radius: Theme.radius
                color: Theme.primaryContainer
                clip: true

                Image {
                    anchors.fill: parent
                    source: Backend.currentCoverArt.length > 0
                        ? Backend.coverUrl(Backend.currentCoverArt)
                        : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: false
                    visible: status === Image.Ready
                }
            }

            Column {
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.03

                Text {
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "Nothing playing"
                    color: Theme.onSurface
                    font.pixelSize: root.height * 0.18
                }
                Text {
                    text: Backend.currentArtist
                    color: Theme.onSurfaceVariant
                    font.pixelSize: root.height * 0.15
                }
            }
        }

        Item {
            Layout.fillWidth: true
        }

        // Transport.
        RowLayout {
            spacing: root.height * 0.05
            Layout.alignment: Qt.AlignVCenter

            IconButton {
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                symbol: "⤨"
                symbolRatio: 0.55
            }
            IconButton {
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                symbol: "⏮"
                symbolRatio: 0.6
                onClicked: Backend.playerPrevious()
            }
            IconButton {
                Layout.preferredWidth: root.height * 0.56
                Layout.preferredHeight: root.height * 0.56
                symbol: Backend.playbackState === "playing" ? "❚❚" : "▶"
                symbolRatio: 0.6
                accent: true
                onClicked: Backend.playerToggle()
            }
            IconButton {
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                symbol: "⏭"
                symbolRatio: 0.6
                onClicked: Backend.playerNext()
            }
            IconButton {
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                symbol: "■"
                symbolRatio: 0.5
                onClicked: Backend.playerStop()
            }
        }

        Item {
            Layout.fillWidth: true
        }

        // Time and volume.
        RowLayout {
            spacing: root.height * 0.12
            Layout.alignment: Qt.AlignVCenter

            Text {
                text: root.formatTime(Backend.position) + " / " + root.formatTime(Backend.duration)
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.15
            }

            Text {
                text: "🔊"
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.18
            }

            Rectangle {
                Layout.preferredWidth: root.width * 0.07
                Layout.preferredHeight: Math.max(1, root.height * 0.05)
                color: Theme.outlineVariant

                Rectangle {
                    width: parent.width * 0.6
                    height: parent.height
                    color: Theme.primary
                }
            }
        }
    }
}
