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

// Transient error/notification banner.
Rectangle {
    id: root

    property string message: ""
    property string kind: ""
    property bool shown: false
    // Motion: quick in, lazy out (nothing keyboard-critical blocks on it).
    property real fadeDuration: Theme.motion.quick

    readonly property bool syncing: kind === "sync"

    signal retry()

    color: syncing ? Palette.tertiaryContainer : Palette.errorContainer
    radius: Theme.radius
    border.width: Math.max(1, height * Theme.hairlineRatio)
    border.color: syncing ? Palette.tertiary : Palette.error
    opacity: shown ? 1 : 0
    visible: opacity > 0

    Behavior on opacity {
        NumberAnimation { duration: root.fadeDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
    }

    function show(kind, message) {
        root.kind = kind;
        root.message = message;
        root.fadeDuration = Theme.motion.quick;
        root.shown = true;
        hideTimer.restart();
    }

    Timer {
        id: hideTimer
        interval: 6000
        onTriggered: {
            root.fadeDuration = Theme.motion.lazy;
            root.shown = false;
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.03
        anchors.rightMargin: root.width * 0.03
        spacing: root.width * 0.03

        FadeText {
            text: "⚠"
            color: root.syncing ? Palette.onTertiaryContainer : Palette.onErrorContainer
            font.pixelSize: Theme.clamp(root.height * 0.36, 12, 18)
            animated: false
        }

        FadeText {
            Layout.fillWidth: true
            text: root.message
            color: root.syncing ? Palette.onTertiaryContainer : Palette.onErrorContainer
            font.pixelSize: Theme.clamp(root.height * 0.3, 12, 18)
            elide: Text.ElideRight
            animated: false
        }

        Rectangle {
            visible: root.syncing
            implicitWidth: retryLabel.implicitWidth + root.height * 0.5
            implicitHeight: root.height * 0.62
            radius: Theme.radius
            color: retryHover.hovered ? Palette.primary : Palette.primaryContainer

            FadeText {
                id: retryLabel
                anchors.centerIn: parent
                text: "Retry"
                color: Palette.onPrimaryContainer
                font.pixelSize: Theme.clamp(root.height * 0.28, 10, 14)
                animated: false
            }

            HoverHandler {
                id: retryHover
                cursorShape: Qt.PointingHandCursor
            }
            TapHandler {
                onTapped: {
                    root.shown = false;
                    root.retry();
                }
            }
        }
    }
}
