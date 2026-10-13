import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Inferior bar: the playback controls, driven by the ffplay engine.
Rectangle {
    id: root

    color: Palette.surfaceContainer

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        var minutes = Math.floor(total / 60);
        var rest = total % 60;
        return minutes + ":" + (rest < 10 ? "0" : "") + rest;
    }

    // Scoped controls while the player bar is the active region.
    function handleAction(action) {
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
        color: Palette.outlineVariant

        Rectangle {
            width: parent.width * (Backend.duration > 0
                ? Math.min(1, Backend.position / Backend.duration)
                : 0)
            height: parent.height
            color: Palette.primary

            Behavior on width {
                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Linear }
            }
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

    Item {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.013
        anchors.rightMargin: root.width * 0.013

        // Current track (left).
        RowLayout {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            spacing: root.height * 0.12

            Rectangle {
                Layout.preferredWidth: root.height * 0.62
                Layout.preferredHeight: root.height * 0.62
                radius: Theme.radius
                color: Palette.primaryContainer
                clip: true

                Image {
                    anchors.fill: parent
                    source: Backend.currentCoverUrl
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: false
                    visible: status === Image.Ready
                }
            }

            Column {
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.03

                FadeText {
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "Nothing playing"
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.18
                    animated: false
                }
                FadeText {
                    text: Backend.currentArtist
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.15
                    animated: false
                }
            }
        }

        // Transport, centred on the bar: play/pause is the visual anchor.
        RowLayout {
            anchors.horizontalCenter: parent.horizontalCenter
            anchors.verticalCenter: parent.verticalCenter
            spacing: root.height * 0.05

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
                symbol: Backend.playbackState === "playing" ? "⏸" : "▶"
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

        // Time and volume (right).
        RowLayout {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: root.height * 0.12

            FadeText {
                text: root.formatTime(Backend.position) + " / " + root.formatTime(Backend.duration)
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.15
                animated: false
            }

            FadeText {
							text: "VOL: "
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.18
                animated: false
            }

            Rectangle {
                Layout.preferredWidth: root.width * 0.07
                Layout.preferredHeight: Math.max(1, root.height * 0.05)
                color: Palette.outlineVariant

                Rectangle {
                    width: parent.width * 0.6
                    height: parent.height
                    color: Palette.primary
                }
            }
        }
    }
}
