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

// Playback node: cover, title, album, progress and play/pause state.
Rectangle {
    id: root

    color: Palette.surfaceContainerHigh
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
            return Palette.primary;
        case "paused":
            return Palette.tertiary;
        default:
            return Palette.onSurfaceVariant;
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
                color: Palette.primaryContainer
                clip: true

                CoverImage {
                    anchors.fill: parent
                    source: Backend.currentCoverUrl
                }
            }

            // Title / album / artist.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: root.width * 0.012

                FadeText {
                    Layout.fillWidth: true
                    text: "MUSIKA"
                    color: Palette.onSurface
                    font.pixelSize: root.width * 0.078
                    font.bold: true
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.width * 0.062
                    elide: Text.ElideRight
                    visible: text.length > 0
                }
                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist
                    color: Palette.onSurfaceVariant
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
            color: Palette.outlineVariant

            Rectangle {
                width: parent.width * (Backend.duration > 0
                    ? Math.min(1, Backend.position / Backend.duration)
                    : 0)
                height: parent.height
                color: root.stateColor()

                Behavior on width {
                    NumberAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
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
                color: Palette.onSurfaceVariant
                font.pixelSize: root.width * 0.058
            }
        }
    }
}
