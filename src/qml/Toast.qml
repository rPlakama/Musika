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

// Transient error/notification banner (UI-55: an acrylic surface). The
// message is never cut: the banner wraps and grows with its text (up to
// `maxWidth`/`maxHeight`), and scrolls when even the full height is not
// enough. Short messages keep the compact width.
AcrylicPanel {
    id: root

    property string message: ""
    property string kind: ""
    property bool shown: false
    // Motion: quick in, lazy out (nothing keyboard-critical blocks on it).
    property real fadeDuration: Theme.motion.quick
    /// Growth caps; hosts size these to the space above the player bar.
    property real maxWidth: parent ? parent.width * 0.96 : width
    property real maxHeight: parent ? parent.height * 0.9 : height
    /// Width the banner keeps for short messages before it starts growing.
    property real compactWidth: parent ? parent.width * 0.42 : 0

    readonly property bool syncing: kind === "sync"
    // VI-8a/logs: informational notices — neutral tint, no retry.
    readonly property bool notice: kind === "log" || kind === "clipboard"
    // UI-23: playback errors get a retry too.
    readonly property bool retryable: kind === "sync" || kind === "playback"

    signal retry()

    // Stable size unit derived from the width cap — never from the growing
    // height — so text, glyph and button size themselves without a loop.
    readonly property real unit: Theme.clamp(root.maxWidth * 0.024, 12, 18)
    readonly property real pad: Math.max(10, root.unit * 0.9)
    readonly property real gap: root.unit * 0.9
    readonly property real iconWidth: root.unit * 1.3
    readonly property real retryWidth: root.retryable
        ? retryLabel.implicitWidth + root.unit * 1.6 : 0
    readonly property real chrome: root.pad * 2 + root.iconWidth + root.gap * 2 + root.retryWidth

    FontMetrics {
        id: metrics
        font: messageText.font
    }

    // Grows with the message: the unwrapped advance width plus chrome, capped
    // at `maxWidth`; the wrapped text then drives the height (capped too).
    implicitWidth: Math.min(root.maxWidth,
        Math.max(root.compactWidth, metrics.advanceWidth(root.message) + root.chrome))
    implicitHeight: Math.min(root.maxHeight, scroller.contentHeight + root.pad * 2)

    tint: notice ? Palette.secondaryContainer
        : (syncing ? Palette.tertiaryContainer : Palette.errorContainer)
    radius: Theme.radius
    border.width: Math.max(1, root.unit * 0.08)
    border.color: notice ? Palette.secondary
        : (syncing ? Palette.tertiary : Palette.error)
    opacity: shown ? 1 : 0
    visible: opacity > 0

    Behavior on opacity { Anim { duration: root.fadeDuration } }

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

    // Scrolls the (wrapped) message when it cannot fit even at `maxHeight`;
    // while it fits, wheel events pass through to the list underneath.
    Flickable {
        id: scroller
        anchors.fill: parent
        anchors.margins: root.pad
        contentWidth: width
        contentHeight: row.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        interactive: contentHeight > height

        RowLayout {
            id: row
            width: scroller.width
            spacing: root.gap

            FadeText {
                text: root.notice ? "✓" : "⚠"
                color: root.notice ? Palette.onSecondaryContainer
                    : (root.syncing ? Palette.onTertiaryContainer : Palette.onErrorContainer)
                font.pixelSize: root.unit * 1.15
                animated: false
            }

            FadeText {
                id: messageText
                Layout.fillWidth: true
                text: root.message
                color: root.notice ? Palette.onSecondaryContainer
                    : (root.syncing ? Palette.onTertiaryContainer : Palette.onErrorContainer)
                font.pixelSize: root.unit
                wrapMode: Text.WordWrap
                animated: false
            }

            Rectangle {
                visible: root.retryable
                implicitWidth: root.retryWidth
                implicitHeight: root.unit * 1.9
                radius: Theme.radius
                color: retryHover.hovered ? Palette.primary : Palette.primaryContainer

                FadeText {
                    id: retryLabel
                    anchors.centerIn: parent
                    text: "Retry"
                    color: Palette.onPrimaryContainer
                    font.pixelSize: Theme.clamp(root.unit * 0.8, 10, 14)
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
}
