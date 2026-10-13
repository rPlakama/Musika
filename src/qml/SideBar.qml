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

// Left sidebar: the library segments. `selected` is this region's cursor and is
// only moved while the sidebar is the active region.
Rectangle {
    id: root

    signal segmentActivated(int index)

    property int selected: 0
    // UI-6: Playlists/Radio are placeholders — visibly disabled, not fake-active.
    readonly property var segments: [
        { label: "Albums", enabled: true },
        { label: "Songs", enabled: true },
        { label: "Artists", enabled: true },
        { label: "Playlists", enabled: false },
        { label: "Radio", enabled: false }
    ]

    color: Palette.surfaceContainerLow

    function moveSelection(delta) {
        var next = selected;
        for (var step = 0; step < segments.length; step++) {
            next = (next + delta + segments.length) % segments.length;
            if (segments[next].enabled)
                break;
        }
        selected = next;
    }

    function handleAction(action) {
        if (action === "move_up")
            moveSelection(-1);
        else if (action === "move_down")
            moveSelection(1);
        else if (action === "enter" && segments[selected].enabled)
            segmentActivated(selected);
    }

    Rectangle {
        id: brand
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: root.height * 0.09
        color: "transparent"

        FadeText {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: root.width * 0.1
            text: "MUSIKA"
            color: Palette.primary
            font.pixelSize: root.height * 0.03
            font.bold: true
            font.letterSpacing: root.height * 0.005
        }
    }

    ColumnLayout {
        anchors.top: brand.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: root.width * 0.07
        spacing: root.height * 0.004

        Repeater {
            model: root.segments

            delegate: Rectangle {
                id: segment
                required property int index
                required property var modelData

                readonly property bool usable: modelData.enabled

                Layout.fillWidth: true
                implicitHeight: root.height * 0.064
                radius: Theme.radius
                color: !usable
                    ? "transparent"
                    : (segment.index === root.selected
                        ? Palette.secondaryContainer
                        : (segmentHover.hovered ? Palette.surfaceContainerHigh : "transparent"))

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                // Accent bar on the selected segment.
                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.width * 0.02
                    color: Palette.primary
                    visible: segment.usable && segment.index === root.selected
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: root.width * 0.1
                    text: segment.modelData.label
                    color: !segment.usable
                        ? Theme.stateDisabledContent
                        : (segment.index === root.selected ? Palette.onSecondaryContainer : Palette.onSurfaceVariant)
                    font.pixelSize: root.height * 0.022
                    font.bold: segment.usable && segment.index === root.selected
                }

                HoverHandler {
                    id: segmentHover
                    enabled: segment.usable
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    enabled: segment.usable
                    onTapped: {
                        root.selected = segment.index;
                        root.segmentActivated(segment.index);
                    }
                }
            }
        }
    }

    // Library quantity statistics (the sync action lives in the top bar).
    ColumnLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: root.width * 0.1
        spacing: root.height * 0.012

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Palette.outlineVariant
        }

        Repeater {
            model: [
                { label: "Artists", value: Backend.artistCount },
                { label: "Albums", value: Backend.albumCount },
                { label: "Songs", value: Backend.songCount }
            ]

            delegate: RowLayout {
                id: stat
                required property var modelData
                Layout.fillWidth: true

                FadeText {
                    Layout.fillWidth: true
                    text: stat.modelData.label
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                }
                FadeText {
                    text: stat.modelData.value
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.02
                    font.bold: true
                }
            }
        }
    }
}
