import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Playback node: cover, title, album, progress and play/pause state.
Rectangle {
    id: root

    color: Theme.surfaceContainerHigh
    radius: Theme.radius
    implicitHeight: content.implicitHeight + root.width * 0.12

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        return Math.floor(total / 60) + ":" + ("0" + (total % 60)).slice(-2);
    }

    function stateLabel() {
        switch (Backend.playbackState) {
        case "playing":
            return "▶  Playing";
        case "paused":
            return "❚❚  Paused";
        default:
            return "■  Stopped";
        }
    }

    function stateColor() {
        switch (Backend.playbackState) {
        case "playing":
            return Theme.primary;
        case "paused":
            return Theme.tertiary;
        default:
            return Theme.onSurfaceVariant;
        }
    }

    ColumnLayout {
        id: content
        anchors.fill: parent
        anchors.margins: root.width * 0.06
        spacing: root.width * 0.05

        RowLayout {
            Layout.fillWidth: true
            spacing: root.width * 0.05

            // Cover.
            Rectangle {
                Layout.preferredWidth: root.width * 0.28
                Layout.preferredHeight: root.width * 0.28
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
            }

            // Title / album / artist.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: root.width * 0.012

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "Nothing playing"
                    color: Theme.onSurface
                    font.pixelSize: root.width * 0.078
                    font.bold: true
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Theme.onSurfaceVariant
                    font.pixelSize: root.width * 0.062
                    elide: Text.ElideRight
                    visible: text.length > 0
                }
                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist
                    color: Theme.onSurfaceVariant
                    font.pixelSize: root.width * 0.055
                    elide: Text.ElideRight
                    visible: text.length > 0
                }
            }
        }

        // Progress.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(2, root.width * 0.016)
            color: Theme.outlineVariant

            Rectangle {
                width: parent.width * (Backend.duration > 0
                    ? Math.min(1, Backend.position / Backend.duration)
                    : 0)
                height: parent.height
                color: root.stateColor()

                Behavior on width {
                    NumberAnimation {
                        duration: Theme.duration
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.easeOut
                    }
                }
            }
        }

        // State + time.
        RowLayout {
            Layout.fillWidth: true

            FadeText {
                text: root.stateLabel()
                color: root.stateColor()
                font.pixelSize: root.width * 0.058
                font.bold: true
            }
            Item {
                Layout.fillWidth: true
            }
            FadeText {
                text: root.formatTime(Backend.position) + " / " + root.formatTime(Backend.duration)
                color: Theme.onSurfaceVariant
                font.pixelSize: root.width * 0.058
            }
        }
    }
}
