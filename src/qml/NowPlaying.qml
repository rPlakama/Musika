import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// "Mainscreen" fullscreen mode: cover on left, lyrics/details on right, with playback transport.
Rectangle {
    id: root

    color: Theme.background

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        var minutes = Math.floor(total / 60);
        var rest = total % 60;
        return minutes + ":" + (rest < 10 ? "0" : "") + rest;
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.03
        spacing: root.height * 0.02

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: root.width * 0.03

            // Cover + track info. Pane split is 50/50: cover left, lyrics right.
            ColumnLayout {
                Layout.fillHeight: true
                Layout.preferredWidth: root.width * 0.5
                spacing: root.height * 0.02

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
                        visible: Backend.currentCoverUrl.length > 0
                    }

                    FadeText {
                        anchors.centerIn: parent
                        text: "MUSIKA"
                        color: Theme.onPrimaryContainer
                        font.pixelSize: root.height * 0.03
                        font.letterSpacing: root.height * 0.008
                        visible: Backend.currentCoverUrl.length === 0
                    }
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "No track playing"
                    color: Theme.onSurface
                    font.pixelSize: Theme.clamp(root.height * 0.036, 18, 32)
                    font.bold: true
                    elide: Text.ElideRight
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist + (Backend.currentAlbum.length > 0 ? "  ·  " + Backend.currentAlbum : "")
                    color: Theme.onSurfaceVariant
                    font.pixelSize: Theme.clamp(root.height * 0.022, 12, 18)
                    elide: Text.ElideRight
                }
            }

            // Lyrics / Details pane
            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.surfaceContainerLow
                radius: Theme.radius

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: root.width * 0.02
                    spacing: root.height * 0.015

                    RowLayout {
                        Layout.fillWidth: true

                        FadeText {
                            text: "Lyrics"
                            color: Theme.primary
                            font.pixelSize: root.height * 0.024
                            font.bold: true
                            font.letterSpacing: root.height * 0.004
                        }

                        Item { Layout.fillWidth: true }

                        FadeText {
                            text: Backend.queueLength > 0 ? "Queue #" + (Backend.queueIndex + 1) + " of " + Backend.queueLength : ""
                            color: Theme.onSurfaceVariant
                            font.pixelSize: root.height * 0.018
                        }
                    }

                    Flickable {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        contentWidth: width
                        contentHeight: lyrics.implicitHeight
                        boundsBehavior: Flickable.StopAtBounds

                        FadeText {
                            id: lyrics
                            width: parent.width
                            text: "In the deepest ocean\nThe bottom of the sea\nYour eyes\nThey turn me\n\nWhy should I stay here?\nWhy should I stay?\n\nI'd be crazy not to follow\nFollow where you lead\nYour eyes\nThey turn me\n\nTurn me on to phantoms\nI follow to the edge of the earth\nAnd fall off\n\nEverybody leaves\nIf they get the chance\nAnd this is my chance"
                            color: Theme.onSurfaceVariant
                            font.pixelSize: root.height * 0.024
                            lineHeight: 1.4
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }

        // Fullscreen transport bar: seek row on top, transport centred below.
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: root.height * 0.1
            color: Theme.surfaceContainer
            radius: Theme.radius

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: root.height * 0.012
                spacing: root.height * 0.006

                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.width * 0.015

                    FadeText {
                        text: root.formatTime(Backend.position)
                        color: Theme.onSurfaceVariant
                        font.pixelSize: Theme.clamp(root.height * 0.02, 10, 14)
                    }

                    Rectangle {
                        id: fsSeekTrack
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(2, root.height * 0.008)
                        color: Theme.outlineVariant
                        radius: Theme.radius

                        Rectangle {
                            width: fsSeekTrack.width * (Backend.duration > 0 ? Math.min(1, Backend.position / Backend.duration) : 0)
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
                        font.pixelSize: Theme.clamp(root.height * 0.02, 10, 14)
                    }
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    // Transport centred on the bar; play/pause is the anchor.
                    RowLayout {
                        anchors.horizontalCenter: parent.horizontalCenter
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: root.width * 0.01
                        IconButton {
                            Layout.preferredWidth: root.height * 0.05
                            Layout.preferredHeight: root.height * 0.05
                            symbol: "⏮"
                            onClicked: Backend.playerPrevious()
                        }
                        IconButton {
                            Layout.preferredWidth: root.height * 0.056
                            Layout.preferredHeight: root.height * 0.056
                            symbol: Backend.playbackState === "playing" ? "⏸" : "▶"
                            accent: true
                            onClicked: Backend.playerToggle()
                        }
                        IconButton {
                            Layout.preferredWidth: root.height * 0.05
                            Layout.preferredHeight: root.height * 0.05
                            symbol: "⏭"
                            onClicked: Backend.playerNext()
                        }
                    }

                    FadeText {
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Esc to exit fullscreen"
                        color: Theme.onSurfaceVariant
                        font.pixelSize: Theme.clamp(root.height * 0.016, 10, 14)
                    }
                }
            }
        }
    }
}
