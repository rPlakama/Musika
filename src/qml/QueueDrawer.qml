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
        content.goto(cursor);
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`).
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow

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
                duration: Theme.formatTime(track.duration),
                cover: track.cover,
                songId: track.id,
                albumId: track.album_id,
                starred: track.starred === true
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
        // VI-7: the count register rides the drawer's moves (3j, 10k, 5G).
        var steps = Math.max(1, Backend.keyCount);
        if (action === "move_up")
            cursor = Math.max(0, cursor - steps);
        else if (action === "move_down")
            cursor = Math.min(items.count - 1, cursor + steps);
        else if ((action === "top" || action === "bottom") && items.count > 0) {
            content.edgeJump(action === "bottom", steps);
            cursor = content.currentIndex;
        } else if ((action === "page_up" || action === "page_down")
                && items.count > 0) {
            content.pageJump(action === "page_down");
            cursor = content.currentIndex;
        } else if (action.indexOf("extend_") === 0 && items.count > 0) {
            content.extendSelection("move_" + action.substring(7));
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
        // Skips dedupe the payload backend-side; the playing marker follows
        // the property instead (no cursor move — parity with later payloads).
        function onQueue_index_changed() {
            root.nowPlaying = Backend.queueIndex;
        }
        // The shared row's heart: patch the matching rows in place, like the
        // fullscreen queue (a heart write must not rebuild the model).
        function onFavoriteChanged(kind, id, on) {
            if (kind !== "song")
                return;
            for (var i = 0; i < items.count; ++i) {
                if (items.get(i).songId === id)
                    items.setProperty(i, "starred", on === true);
            }
        }
    }

    // UI-35: the drawer owns the context while open — the shared scrim dims
    // whatever is behind, and a click outside closes (search-overlay parity).
    Rectangle {
        radius: Theme.radius
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
                // ARC-UI-2 M1: the rendered cells.
                searchFields: ["title", "duration"]
                // UI-58: rows scale from the drawer panel height; the
                // precomputed extent keeps the shared glide off Qt's lazy
                // estimate.
                rowRatio: 0.06
                rowRefHeight: panel.height

                Behavior on opacity {
                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                }

                rowDelegate: Component {
                    Rectangle {
                        radius: Theme.radius
                        id: row
                        required property int index
                        required property string title
                        required property string duration
                        required property string cover
                        required property string songId
                        required property string albumId
                        required property bool starred

                        readonly property bool emphasized: index === root.cursor
                            || rowHover.hovered

                        width: ListView.view.width
                        // UI-58: the model's panel-height adaptive row.
                        height: content.adaptiveRowHeight
                        color: index === root.cursor
                            ? Palette.secondaryContainer
                            : (index === root.nowPlaying ? Palette.surfaceContainer : "transparent")

                        // The shared list row (cover context, like the tabs).
                        RowBody {
                            anchors.fill: parent
                            viewWidth: row.width
                            rowHeight: row.height
                            emphasized: row.emphasized
                            showCover: true
                            showSubtitle: false
                            cover: row.cover
                            title: row.index === root.nowPlaying
                                ? "▶ " + row.title
                                : row.title
                            meta: row.duration
                            titlePixelSize: row.height * 0.34
                            metaPixelSize: row.height * 0.28
                            titleColor: row.index === root.cursor
                                ? Palette.onSecondaryContainer
                                : Palette.onSurface
                            reserveMeta: true
                            metaAlignRight: true
                            leftMargin: row.width * 0.012
                            textGap: row.height * 0.16
                            rightMargin: row.height * 0.1
                            marksMargin: row.width * 0.01275
                            markCursor: root.cursor
                            visibleStart: content.firstVisibleIndex
                            visibleEnd: content.lastVisibleIndex
                            markIndex: row.index
                            markSongId: row.songId
                            markAlbumId: row.albumId
                            markStarred: row.starred
                            searchQuery: content.searchQuery
                            searchCurrent: row.index === content.searchCurrentRow
                        }

                        HoverHandler {
                            id: rowHover
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            acceptedModifiers: Qt.ShiftModifier
                            onTapped: {
                                content.extendSelectionTo(row.index);
                                root.cursor = content.currentIndex;
                            }
                        }
                        TapHandler {
                            acceptedModifiers: Qt.NoModifier
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
