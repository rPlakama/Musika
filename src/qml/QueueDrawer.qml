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
    // UI-55: the item behind the drawer whose pixels feed the acrylic blur.
    property Item backdrop: null
    // A reorder round-trip re-emits the queue; don't crossfade that one.
    property bool _reordering: false
    // UI-54: first queue payload after opening restores the saved highlight.
    property bool _recall: false

    // Keep rendering while the close slide runs (the panel owns the x).
    visible: opened || panel.x < root.width
    z: 25

    function open() {
        opened = true;
        root._recall = true;
        cursor = Math.max(0, nowPlaying);
        Backend.requestQueue();
        Backend.releaseFocus();
    }

    function close() {
        if (items.count > 0)
            content.remember();
        opened = false;
        Backend.releaseFocus();
    }

    function toggle() {
        if (opened)
            close();
        else
            open();
    }

    function centerPlaying(index) {
        cursor = Math.max(0, Math.min(items.count - 1, index));
        content.currentIndex = cursor;
        content.view.positionViewAtIndex(cursor, ListView.Contain);
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`).
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow

    /// UI-58: the row baseline (0.06 of the panel), captured on the panel's
    /// first real measure so the adaptive rule preserves the default size.
    property real _rowBase: 0

    Connections {
        target: panel
        function onHeightChanged() {
            if (root._rowBase <= 0 && panel.height > 0)
                root._rowBase = panel.height * 0.06;
        }
    }

    function searchRows() {
        return content.searchRows();
    }

    function searchJump(row) {
        content.searchJump(row);
        cursor = content.currentIndex;
    }

    function apply(payload) {
        var data = JSON.parse(payload);
        // UI-41: a payload swap fades in quickly instead of flashing; a
        // reorder keeps the list live.
        if (!root._reordering)
            content.opacity = 0;
        root._reordering = false;
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
        // UI-54: restore the highlight where the user left it; a fresh drawer
        // opens on the playing track.
        if (!root._recall || !content.restore())
            centerPlaying(data.index);
        else
            cursor = content.currentIndex;
        root._recall = false;
        Qt.callLater(function() { content.opacity = 1; });
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
            content.edgeJump(action === "bottom");
            cursor = content.currentIndex;
        } else if (action === "enter")
            Backend.queueJump(cursor);
        else if (action === "queue_up" && cursor > 0) {
            root._reordering = true;
            Backend.queueMove(cursor, cursor - 1);
            cursor -= 1;
        } else if (action === "queue_down" && cursor < items.count - 1) {
            root._reordering = true;
            Backend.queueMove(cursor, cursor + 1);
            cursor += 1;
        } else if (action === "queue_remove")
            Backend.queueRemove(cursor);

        // The view follows cursor changes itself; gg/G snap via edgeJump.
        content.currentIndex = cursor;
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

    // UI-35: the drawer owns the context while open — the shared scrim dims
    // whatever is behind, and a click outside closes (search-overlay parity).
    Rectangle {
        anchors.fill: parent
        color: Palette.scrim
        opacity: root.opened ? Theme.scrimOpacity : 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }

        TapHandler {
            enabled: root.opened
            onTapped: root.close()
        }
    }

    AcrylicPanel {
        id: panel
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.width * 0.36
        tint: Palette.surfaceContainerHigh
        backdrop: root.backdrop
        border.color: Palette.outlineVariant
        border.width: Math.max(1, root.width * Theme.hairlineRatio)
        // UI-35: slide in/out from the right edge (`standard`); the root stays
        // visible while the slide-out runs.
        x: root.opened ? root.width - width : root.width

        Behavior on x {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
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

            ScrollContent {
                id: content
                Layout.fillWidth: true
                Layout.fillHeight: true
                model: items
                memoryKey: "queue"
                // ARC-UI-2 M1: the rendered cells (the `subtitle` role is
                // never painted, so it never matches).
                searchFields: ["title", "duration"]

                Behavior on opacity {
                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                }

                rowDelegate: Component {
                    Rectangle {
                        id: row
                    required property int index
                    required property string title
                    required property string subtitle
                    required property string duration
                    required property string cover

                    readonly property bool emphasized: index === root.cursor || rowHover.hovered

                    width: ListView.view.width
                    // UI-58: adaptive — cut a row past the 18% squeeze.
                    height: Theme.adaptiveRow(panel.height, 0.06, root._rowBase)
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
                        // UI-45: the art grows with the emphasized row (same
                        // animation as the title, transform only).
                        scale: (row.emphasized && !Theme.reduceMotion) ? Theme.selectionEmphasis : 1.0

                        Behavior on scale {
                            NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                        }

                        CoverImage {
                            anchors.fill: parent
                            source: row.cover
                            // ARC-40: ×2 headroom for the UI-45 hover scale.
                            sizeFactor: 2
                        }
                    }

                    SearchText {
                        id: titleText
                        anchors.left: thumb.right
                        anchors.leftMargin: row.height * 0.16
                        anchors.right: durationLabel.left
                        anchors.rightMargin: row.height * 0.16
                        anchors.verticalCenter: parent.verticalCenter
                        text: index === root.nowPlaying ? "▶ " + row.title : row.title
                        color: index === root.cursor ? Palette.onSecondaryContainer : Palette.onSurface
                        font.pixelSize: row.height * 0.34
                        elide: Text.ElideRight
                        query: content.searchQuery
                        currentMatch: index === content.searchCurrentRow
                        transformOrigin: Item.Left
                        // UI-45: the cursor/hover row leads, capped to its box.
                        scale: (row.emphasized && !Theme.reduceMotion)
                            ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                            : 1.0

                        Behavior on scale {
                            NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                        }
                    }

                    SearchText {
                        id: durationLabel
                        anchors.right: parent.right
                        anchors.rightMargin: row.height * 0.1
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.duration
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.28
                        query: content.searchQuery
                        currentMatch: index === content.searchCurrentRow
                    }

                    HoverHandler {
                        id: rowHover
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
