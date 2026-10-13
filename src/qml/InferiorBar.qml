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
                iconSource: "qrc:/app/icons/random-svgrepo-com.svg"
                symbolRatio: 0.65
            }
            IconButton {
                id: previousButton
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                iconSource: "qrc:/app/icons/skip-previous-solid-svgrepo-com.svg"
                symbolRatio: 0.9
                onClicked: Backend.playerPrevious()
            }
            IconButton {
                Layout.preferredWidth: root.height * 0.56
                Layout.preferredHeight: root.height * 0.56
                symbolRatio: 0.6
                iconSource: Backend.playbackState === "playing"
                    ? "qrc:/app/icons/pause-svgrepo-com.svg"
                    : "qrc:/app/icons/play-1001-svgrepo-com.svg"
                symbolColor: Theme.iconStrong
                onClicked: Backend.playerToggle()
            }
            IconButton {
                id: nextButton
                Layout.preferredWidth: root.height * 0.46
                Layout.preferredHeight: root.height * 0.46
                iconSource: "qrc:/app/icons/skip-next-circle-solid-svgrepo-com.svg"
                symbolRatio: 0.9
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

        // Skip feedback: fires for keyboard, mouse and IPC skips alike, in the
        // generated primary.
        Connections {
            target: Backend
            function onTrackSkipped(forward) {
                if (forward)
                    nextButton.bloom();
                else
                    previousButton.bloom();
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
