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

    // UI-57: the shared width ramp — the now-playing type shrinks with the
    // window instead of only with its height.
    readonly property real widthScale: Theme.widthScale(root.width)
    // UI-57: every text in the bar rides the shared type ramp — the role's
    // floor is the hard stop, so a narrow (or short) bar never squeezes the
    // title, the artist or the timecode into illegibility.
    readonly property real titlePixelSize: {
        // The title reads 60% larger than the shared label floor.
        var full = Theme.fontSize("label", root.height) * 1.6;
        return Theme.clamp(full * root.widthScale, 16, full);
    }
    readonly property real artistPixelSize: titlePixelSize
    readonly property real timePixelSize: {
        var full = Theme.fontSize("mono", root.height);
        return Theme.clamp(full * root.widthScale, 10, full);
    }

    // Speaker icon by volume tier: muted / silent / low (≤33) / high.
    function volumeIcon() {
        return Theme.volumeIcon(Backend.volume, Backend.muted);
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
        cursorShape: Qt.PointingHandCursor

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

        // Three explicit zones share the bar and can never overlap: the side
        // zones hold equal preferred widths (so the transport stays dead
        // centre), every title/composer lives in its own bounded rectangle and
        // elides with "…" when it cannot fit. The stop control is gone;
        // `Backend.playerStop` stays for MPRIS, which requires the verb.
        RowLayout {
            anchors.fill: parent
            spacing: root.width * 0.013

            // Left zone: cover + title/composer boxes.
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: root.width * 0.32
                Layout.alignment: Qt.AlignVCenter
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

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    spacing: root.height * 0.03

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: titleText.implicitHeight
                        color: "transparent"

                        FadeText {
                            id: titleText
                            anchors.fill: parent
                            text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "Nothing playing"
                            color: Palette.onSurface
                            font.pixelSize: root.titlePixelSize
                            // The empty state centers in its parent rectangle
                            // (no art to align under); a real title keeps the
                            // left rail, both vertically centered.
                            horizontalAlignment: Backend.currentTitle.length > 0
                                ? Text.AlignLeft : Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                            elide: Text.ElideRight
                            animated: false
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: artistText.implicitHeight
                        color: "transparent"

                        FadeText {
                            id: artistText
                            anchors.fill: parent
                            text: Backend.currentArtist
                            color: Palette.onSurfaceVariant
                            font.pixelSize: root.artistPixelSize
                            elide: Text.ElideRight
                            animated: false
                        }
                    }
                }
            }

            // Centre zone: transport rectangles (previous, pause/play, skip).
            RowLayout {
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.05

                IconButton {
                    id: previousButton
                    Layout.preferredWidth: root.height * 0.46
                    Layout.preferredHeight: root.height * 0.46
                    iconSource: "qrc:/app/icons/skip-previous-solid-svgrepo-com.svg"
                    symbolRatio: 0.9
                    onClicked: Backend.playerPrevious()
                }
                IconButton {
                    id: playButton
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
            }

            // Right zone: elapsed/duration + volume, right-aligned; the time
            // box shrinks (elides) before the volume cluster can collide.
            RowLayout {
                Layout.fillWidth: true
                Layout.preferredWidth: root.width * 0.32
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.12

                FadeText {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
                    text: Theme.formatTime(Backend.position) + " / " + Theme.formatTime(Backend.duration)
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.timePixelSize
                    horizontalAlignment: Text.AlignRight
                    elide: Text.ElideRight
                    animated: false
                }

                IconButton {
                    Layout.preferredWidth: root.height * 0.34
                    Layout.preferredHeight: root.height * 0.34
                    iconSource: root.volumeIcon()
                    symbolRatio: 0.95
                    onClicked: Backend.playerToggleMute()
                }

                Rectangle {
                    id: volumeTrack
                    Layout.preferredWidth: root.width * 0.07
                    Layout.preferredHeight: Math.max(2, root.height * 0.05)
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

        // Skip/play feedback: fires for keyboard, mouse and IPC alike.
        Connections {
            target: Backend
            function onTrackSkipped(forward) {
                if (forward)
                    nextButton.press();
                else
                    previousButton.press();
            }
            function onPlayToggled() {
                playButton.press();
            }
        }
    }
}
