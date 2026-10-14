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

// Queue drawer (Ctrl-Q): the current play queue as a list. j/k move the cursor,
// Enter jumps, Shift-J/Shift-K reorder, dd removes, Esc closes.
Item {
    id: root

    property bool opened: false
    property int cursor: 0
    property int nowPlaying: -1

    // Keep rendering while the close fade runs (the panel owns the opacity).
    visible: opened || panel.opacity > 0
    z: 25

    function open() {
        opened = true;
        cursor = Math.max(0, nowPlaying);
        Backend.requestQueue();
        Backend.releaseFocus();
    }

    function close() {
        opened = false;
        Backend.releaseFocus();
    }

    function toggle() {
        if (opened)
            close();
        else
            open();
    }

    function apply(payload) {
        var data = JSON.parse(payload);
        items.clear();
        for (var i = 0; i < data.items.length; i++) {
            var track = data.items[i];
            items.append({
                title: track.title,
                subtitle: (track.artist ? track.artist : "") + (track.album ? " · " + track.album : ""),
                duration: Theme.formatTime(track.duration),
                cover: track.cover
            });
        }
        nowPlaying = data.index;
        cursor = Math.max(0, Math.min(items.count - 1, data.index));
        list.currentIndex = cursor;
        list.positionViewAtIndex(cursor, ListView.Contain);
    }

    function handleAction(action) {
        if (action === "back" || action === "queue_toggle") {
            close();
            return;
        }
        if (action === "move_up")
            cursor = Math.max(0, cursor - 1);
        else if (action === "move_down")
            cursor = Math.min(items.count - 1, cursor + 1);
        else if ((action === "top" || action === "bottom") && items.count > 0) {
            Theme.cursorEdge(list, action === "bottom", ListView.Contain);
            cursor = list.currentIndex;
        } else if (action === "enter")
            Backend.queueJump(cursor);
        else if (action === "queue_up" && cursor > 0) {
            Backend.queueMove(cursor, cursor - 1);
            cursor -= 1;
        } else if (action === "queue_down" && cursor < items.count - 1) {
            Backend.queueMove(cursor, cursor + 1);
            cursor += 1;
        } else if (action === "queue_remove")
            Backend.queueRemove(cursor);

        list.currentIndex = cursor;
        list.positionViewAtIndex(cursor, ListView.Contain);
    }

    ListModel {
        id: items
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "queue" && root.opened)
                root.apply(payload);
        }
    }

    Rectangle {
        id: panel
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.right: parent.right
        width: root.width * 0.36
        color: Palette.surfaceContainerHigh
        border.color: Palette.outlineVariant
        border.width: Math.max(1, root.width * Theme.hairlineRatio)
        opacity: root.opened ? 1 : 0

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: panel.width * 0.05
            spacing: panel.height * 0.012

            FadeText {
                Layout.fillWidth: true
                text: "Queue"
                color: Palette.primary
                font.pixelSize: panel.width * 0.07
                font.bold: true
            }

            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: items
                boundsBehavior: Flickable.StopAtBounds

                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string title
                    required property string subtitle
                    required property string duration
                    required property string cover

                    width: list.width
                    height: panel.height * 0.06
                    color: index === root.cursor
                        ? Palette.secondaryContainer
                        : (index === root.nowPlaying ? Palette.surfaceContainer : "transparent")

                    Rectangle {
                        id: thumb
                        anchors.left: parent.left
                        anchors.verticalCenter: parent.verticalCenter
                        width: row.height * 0.72
                        height: row.height * 0.72
                        radius: Theme.radius
                        color: Palette.surfaceContainerHighest
                        clip: true

                        CoverImage {
                            anchors.fill: parent
                            source: row.cover
                        }
                    }

                    FadeText {
                        anchors.left: thumb.right
                        anchors.leftMargin: row.height * 0.16
                        anchors.right: durationLabel.left
                        anchors.rightMargin: row.height * 0.16
                        anchors.verticalCenter: parent.verticalCenter
                        text: index === root.nowPlaying ? "▶ " + row.title : row.title
                        color: index === root.cursor ? Palette.onSecondaryContainer : Palette.onSurface
                        font.pixelSize: row.height * 0.34
                        elide: Text.ElideRight
                    }

                    FadeText {
                        id: durationLabel
                        anchors.right: parent.right
                        anchors.rightMargin: row.height * 0.1
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.duration
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.28
                    }

                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: {
                            root.cursor = row.index;
                            Backend.queueJump(row.index);
                        }
                    }
                }
            }

            FadeText {
                Layout.fillWidth: true
                text: "Enter jump · J/K move · dd remove · Esc close"
                color: Palette.onSurfaceVariant
                font.pixelSize: panel.width * 0.045
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
