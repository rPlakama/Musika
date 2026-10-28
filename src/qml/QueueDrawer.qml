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

// Standalone play-queue drawer (Ctrl-Q), hosted by Home so it can slide over
// the library or the fullscreen player alike. The drawer owns its model,
// cursor, selection, search and removal state; Home only toggles it and
// forwards keys while it is open.
Item {
    id: root

    /// Whether the drawer is slid in.
    property bool open: false
    /// The item behind the panel whose pixels feed the acrylic pass (Home
    /// binds the visible shell or the fullscreen player).
    property Item backdrop: null
    /// ARC-UI-1: Alt-hold quick actions on the row under the cursor; Home
    /// routes it to the shared overlay.
    signal quickAction(var target, var actions)

    property int queueCursor: 0
    property int queueIndex: -1
    // One queue payload after a reorder keeps the moved cursor instead of
    // snapping back to the playing track (Q-9).
    property bool _reordering: false
    // `dd` steps the cursor up (a phantom k) and slides the row out before
    // the backend drops it; keep that cursor across the model rebuild.
    property int _removingIndex: -1
    property int _pendingQueueCursor: -1
    // The rows the queue model currently holds: a payload with the same
    // backend row hash is a skip/advance, not a queue change — no rebuild.
    property string _queueHash: ""
    // UI-54: first queue payload after opening restores the saved highlight.
    property bool _recallQueue: false

    Timer {
        id: removeTimer
        repeat: false
        interval: Theme.reduceMotion ? 0 : Theme.motion.standard * 1.3
        onTriggered: root.finishQueueRemove()
    }

    /// Ask the backend for the download state of the rendered rows only.
    function requestMarkWindow(force) {
        var ids = [];
        var lo = queueContent.firstVisibleIndex;
        var hi = queueContent.lastVisibleIndex;
        if (lo < 0 || hi < lo) {
            lo = Math.max(0, root.queueCursor);
            hi = Math.min(queueItems.count - 1, lo);
        }
        for (var i = lo; i <= hi; ++i) {
            var row = queueItems.get(i);
            if (row && row.songId)
                ids.push(row.songId);
        }
        if (force)
            Backend.refreshDownloads(JSON.stringify(ids), "[]");
        else
            Backend.requestDownloads(JSON.stringify(ids), "[]");
    }

    onQueueCursorChanged: root.requestMarkWindow()

    Connections {
        target: queueContent
        function onFirstVisibleIndexChanged() {
            root.requestMarkWindow();
        }
        function onLastVisibleIndexChanged() {
            root.requestMarkWindow();
        }
    }

    function moveQueueCursor(delta) {
        if (queueItems.count === 0)
            return;
        queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor + delta));
        // The view follows cursor changes itself (SCROLLCONTENTSTUDY.md).
        queueContent.currentIndex = queueCursor;
    }

    /// VI-8a: yanks the cursor row (`rowOnly`, vim `Y`) or the visual range
    /// (`y` with a selection). The range ends like vim's visual `y`.
    function yankRows(rowOnly) {
        var indices = (rowOnly || !root.hasSelection)
            ? [root.queueCursor]
            : queueContent.selectionIndices();
        if (indices.length === 0)
            return;
        var ids = [];
        for (var i = 0; i < indices.length; ++i) {
            var item = queueItems.get(indices[i]);
            if (item && item.songId)
                ids.push(item.songId);
        }
        if (ids.length === 0)
            return;
        queueContent.clearSelection();
        Backend.yankRows("song", JSON.stringify(ids));
    }

    /// VI-8a: pastes the row clipboard above (`P`) or below (`p`) the
    /// cursor. The payload rebuild lands the cursor on the first pasted row;
    /// an empty clipboard is a silent no-op.
    function pasteRows(above) {
        if (Backend.clipboardCount <= 0)
            return;
        var at = root.queueCursor + (above ? 0 : 1);
        root._pendingQueueCursor = Math.max(0, Math.min(queueItems.count, at));
        queueContent.clearSelection();
        Backend.pasteQueueRows(root.queueCursor, above);
    }

    function removeQueueRow(index) {
        if (root._removingIndex >= 0 || index < 0 || index >= queueItems.count)
            return;
        root._removingIndex = index;
        queueContent.clearSelection();
        // A real k: step the highlight up before the row disappears, so the
        // rebuild never snaps it back to the playing track.
        root.moveQueueCursor(-1);
        root._pendingQueueCursor = root.queueCursor;
        var row = queueContent.view.itemAtIndex(index);
        if (row) {
            row.removing = true;
            removeTimer.restart();
        } else {
            root.finishQueueRemove();
        }
    }

    function finishQueueRemove() {
        if (root._removingIndex < 0)
            return;
        var index = root._removingIndex;
        root._removingIndex = -1;
        root._pendingQueueCursor = root.queueCursor;
        Backend.queueRemove(index);
    }

    // VI-4: visual selection over the queue. The state lives in
    // `queueContent` (the shared scroll model); Home routes `v`/`V`/Esc here
    // while the drawer is open.
    readonly property bool hasSelection: queueContent.hasSelection
    readonly property int selectionCount: queueContent.selectionCount

    function toggleSelection() {
        queueContent.toggleSelection();
    }

    function selectAll() {
        if (queueItems.count === 0)
            return;
        queueCursor = queueItems.count - 1;
        queueContent.currentIndex = queueCursor;
        queueContent.selectAllRows();
        queueContent.goto(queueCursor);
    }

    function clearSelection() {
        queueContent.clearSelection();
    }

    /// Ctrl-Q: slide the queue in on the playing track (or the remembered
    /// highlight) and keep it live while it is closed.
    function openQueue() {
        root.open = true;
        root._recallQueue = true;
        if (queueItems.count === 0)
            Backend.requestQueue();
        else
            root.recallQueueCursor();
        Backend.releaseFocus();
    }

    function closeQueue() {
        if (queueItems.count > 0)
            queueContent.remember();
        queueContent.clearSelection();
        root.open = false;
        Backend.releaseFocus();
    }

    function toggleQueue() {
        if (root.open)
            root.closeQueue();
        else
            root.openQueue();
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`) — `/` jumps
    /// focus into the drawer and the context-Enter lands on the matched row.
    property alias searchQuery: queueContent.searchQuery
    property alias searchCurrentRow: queueContent.searchCurrentRow

    function searchRows() {
        return queueContent.searchRows();
    }

    function searchJump(row) {
        root.openQueue();
        queueContent.searchJump(row);
        queueCursor = queueContent.currentIndex;
    }

    function applyQueue(payload) {
        var data = JSON.parse(payload);
        var rowsChanged = data.hash !== root._queueHash;

        // A skip or an auto-advance re-delivers the exact same rows with a
        // new playing index. Rebuilding the model for that flicked the pane
        // (clear + fade) and, when the follow raced the refill, corrupted
        // the view's layout (realised rows stopped at the old window and
        // later jumps inflated the item offsets — SC-5). So: rebuild only
        // when the rows really changed; a pure move just follows the index.
        if (rowsChanged) {
            root._queueHash = data.hash;
            // ARC-memory: a different row set is a new queue — drop the old
            // queue's remembered scroll/prefetch target before it can be
            // restored.
            Backend.scrollClear("fullscreen");
            root._recallQueue = false;
            // UI-41: payload swaps fade in quickly; a reorder keeps the list live.
            if (!root._reordering)
                queueContent.opacity = 0;
            queueItems.clear();
            for (var i = 0; i < data.items.length; i++) {
                var track = data.items[i];
                queueItems.append({
                    title: track.title,
                    subtitle: track.subtitle ? track.subtitle : "",
                    duration: track.meta,
                    songId: track.songId,
                    albumId: track.albumId ? track.albumId : "",
                    starred: track.starred === true
                });
            }
            Qt.callLater(function () {
                queueContent.opacity = 1;
            });
        }

        queueIndex = data.index;
        if (root._pendingQueueCursor >= 0 && rowsChanged) {
            root._recallQueue = false;
            queueCursor = Math.max(0, Math.min(queueItems.count - 1, root._pendingQueueCursor));
            root._pendingQueueCursor = -1;
            queueContent.currentIndex = queueCursor;
            queueContent.land(queueCursor);
        } else if (root._reordering) {
            root._reordering = false;
            root._recallQueue = false;
            queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor));
            queueContent.currentIndex = queueCursor;
            queueContent.goto(queueCursor);
        } else {
            // A pure index move is a skip/advance: glide to the new playing
            // row instead of snapping. A rebuilt payload lands instantly.
            root.recallQueueCursor(!rowsChanged);
        }
        root.requestMarkWindow();
    }

    /// The queue cursor for a fresh payload (UI-54 re-entry remembers; a
    /// live payload follows the playing track). Shared by `applyQueue` and
    /// the keep-alive re-entry.
    function recallQueueCursor(animate) {
        if (root._recallQueue && queueContent.restore()) {
            root._recallQueue = false;
            queueCursor = queueContent.currentIndex;
            return;
        }
        root._recallQueue = false;
        queueCursor = Math.max(0, Math.min(queueIndex, Math.max(0, queueItems.count - 1)));
        queueContent.currentIndex = queueCursor;
        if (animate)
            queueContent.goto(queueCursor);
        else
            queueContent.land(queueCursor);
    }

    /// ARC-UI-1: Alt armed the gesture — the open drawer anchors on the row
    /// under the cursor.
    function armQuickActions() {
        if (queueItems.count === 0 || queueContent.currentIndex < 0)
            return;
        var item = queueItems.get(queueContent.currentIndex);
        var row = queueContent.view.itemAtIndex(queueContent.currentIndex);
        var actions = root.hasSelection ? verbs.forSelection("song", queueContent.selectionIds("songId"), queueContent.selectionStarred("songId")) : (item ? verbs.song(item) : null);
        if (!actions)
            return;
        // ARC-I-4: the heart tracks the verb immediately (see `MediaList`).
        if (actions.south && actions.south.invoke) {
            var want = actions.south.favorite === true;
            var star = actions.south.invoke;
            actions.south.invoke = function () {
                star();
                queueContent.markStarred(want);
            };
        }
        if (row)
            root.quickAction(row, actions);
    }

    // The open drawer owns its navigation keys; the caller routes here while
    // `open` is set (Home keeps transport and the modals outside).
    function handleAction(action) {
        // `q` and Ctrl-Q close the drawer; Esc is a vi key here (below).
        if (action === "back" || action === "queue_toggle") {
            root.closeQueue();
            return;
        }
        if (action === "cancel") {
            // Esc is a vi key inside the drawer: it ends the visual selection
            // instead of closing (q / Ctrl-Q close the drawer).
            if (root.hasSelection)
                queueContent.clearSelection();
            return;
        }
        // VI-8a row clipboard (only reachable while the drawer is open).
        if (action === "yank") {
            root.yankRows(false);
            return;
        }
        if (action === "yank_row") {
            root.yankRows(true);
            return;
        }
        if (action === "paste_below") {
            root.pasteRows(false);
            return;
        }
        if (action === "paste_above") {
            root.pasteRows(true);
            return;
        }
        // VI-7: the count walks the queue cursor by that many rows.
        var queueSteps = Math.max(1, Backend.keyCount);
        if (action === "top" || action === "bottom") {
            queueContent.edgeJump(action === "bottom", queueSteps);
            queueCursor = queueContent.currentIndex;
            return;
        }
        if (action === "move_up")
            moveQueueCursor(-queueSteps);
        else if (action === "move_down")
            moveQueueCursor(queueSteps);
        else if (action === "page_up" || action === "page_down") {
            queueContent.pageJump(action === "page_down");
            queueCursor = queueContent.currentIndex;
        } else if (action.indexOf("extend_") === 0) {
            queueContent.extendSelection("move_" + action.substring(7));
            queueCursor = queueContent.currentIndex;
        } else if (action === "queue_up" && queueContent.allows("reorder") && queueCursor > 0) {
            root._reordering = true;
            // VI-4: a reorder shifts every anchored row; the range dies.
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor - 1);
            queueCursor -= 1;
            queueContent.currentIndex = queueCursor;
        } else if (action === "queue_down" && queueContent.allows("reorder") && queueCursor < queueItems.count - 1) {
            root._reordering = true;
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor + 1);
            queueCursor += 1;
            queueContent.currentIndex = queueCursor;
        } else if (action === "queue_remove" && queueContent.allows("remove") && queueCursor >= 0) {
            // VI-4: a visual selection removes the whole range in one pass;
            // a bare `dd` keeps the single-row slide-out.
            if (root.hasSelection) {
                var indices = queueContent.selectionIndices();
                if (indices.length > 0) {
                    root._pendingQueueCursor = indices[0];
                    queueContent.clearSelection();
                    Backend.queueRemoveMany(JSON.stringify(indices));
                }
            } else {
                root.removeQueueRow(queueCursor);
            }
        } else if (action === "enter") {
            // VI-4: jumping to a track ends the visual selection.
            queueContent.clearSelection();
            if (queueCursor >= 0)
                Backend.queueJump(queueCursor);
        }
    }

    // ── Scrim + drawer -----------------------------------------------------
    Rectangle {
        id: queueScrim
        anchors.fill: parent
        color: Palette.scrim
        opacity: root.open ? Theme.scrimOpacity : 0
        visible: opacity > 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }

        TapHandler {
            enabled: root.open
            onTapped: root.closeQueue()
        }
    }

    AcrylicPanel {
        id: queueDrawer
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: root.width * 0.36
        tint: Palette.surfaceContainerHigh
        backdrop: root.backdrop
        border.color: Palette.primary
        border.width: Math.max(1, root.width * Theme.hairlineRatio)
        radius: Theme.radius
        clip: true
        visible: root.open || x < root.width
        x: root.open ? root.width - width : root.width
        z: 1

        Behavior on x {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.open ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: root.width * 0.012
            spacing: root.height * 0.006

            RowLayout {
                Layout.fillWidth: true

                FadeText {
                    text: "Queue"
                    color: Palette.primary
                    font.pixelSize: Theme.fontSize("title", root.height)
                    font.bold: true
                }

                Item {
                    Layout.fillWidth: true
                }

                // VI-4: the selection count while a range is up.
                SelectionBadge {
                    Layout.alignment: Qt.AlignVCenter
                    count: root.selectionCount
                    pixelSize: Theme.fontSize("label", root.height)
                }

                FadeText {
                    text: Backend.queueLength > 0 ? (Backend.queueIndex + 1) + " / " + Backend.queueLength : ""
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("label", root.height)
                }
            }

            Rectangle {
                radius: Theme.radius
                Layout.fillWidth: true
                Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                color: Palette.primary
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                // UI-48: the shared scroll body carries the overflow stage —
                // the highlight bleeds into the gutters while rows still clip
                // vertically at the list bounds.
                ScrollContent {
                    id: queueContent
                    anchors.fill: parent
                    context: "queue"
                    rowRatio: 0.06
                    rowRefHeight: queueDrawer.height
                    model: queueItems
                    selectable: true
                    memoryKey: "queue"
                    // ARC-UI-2 M1: the queue rows paint title and duration.
                    searchFields: ["title", "subtitle", "duration"]
                    visible: queueItems.count > 0

                    Behavior on opacity {
                        NumberAnimation {
                            duration: Theme.motion.quick
                            easing.type: Easing.Bezier
                            easing.bezierCurve: Theme.ease.standard
                        }
                    }

                    rowDelegate: Component {
                        Rectangle {
                            id: row
                            radius: Theme.radius
                            required property int index
                            required property string title
                            required property string subtitle
                            required property string duration
                            required property string songId
                            required property bool starred

                            readonly property bool emphasized: index === root.queueCursor || rowHover.hovered

                            width: ListView.view.width
                            z: emphasized ? 1 : 0
                            // UI-58: adaptive — stops shrinking at the 18%
                            // floor, never jumps a row.
                            height: queueContent.adaptiveRowHeight
                            // Playing-track tint stays on the row; the
                            // cursor/hover block is the shared UI-48 layer.
                            color: index === root.queueIndex ? Palette.surfaceContainer : "transparent"
                            property bool removing: false
                            opacity: removing ? 0 : 1

                            Behavior on opacity {
                                NumberAnimation {
                                    duration: Theme.reduceMotion ? 0 : Theme.motion.standard * 1.3
                                    easing.type: Easing.Bezier
                                    easing.bezierCurve: Theme.ease.standard
                                }
                            }

                            transform: Translate {
                                x: row.removing ? -row.width : 0

                                Behavior on x {
                                    NumberAnimation {
                                        duration: Theme.reduceMotion ? 0 : Theme.motion.standard * 1.3
                                        easing.type: Easing.Bezier
                                        easing.bezierCurve: Theme.ease.standard
                                    }
                                }
                            }

                            RowHighlight {
                                current: index === root.queueCursor
                                hovered: rowHover.hovered
                                selected: queueContent.selected(index)
                                overflow: queueContent.bleed
                            }

                            RowBody {
                                anchors.fill: parent
                                viewWidth: row.width
                                rowHeight: row.height
                                emphasized: row.emphasized
                                current: index === root.queueCursor
                                showCover: false
                                showSubtitle: row.subtitle.length > 0
                                title: row.title
                                subtitle: row.subtitle
                                meta: row.duration
                                titleColor: index === root.queueCursor ? Palette.onSecondaryContainer : Palette.onSurface
                                reserveMeta: true
                                metaAlignRight: true
                                leftMargin: row.width * 0.01
                                textGap: row.width * 0.01
                                rightMargin: 0
                                marksMargin: row.width * 0.00935
                                markCursor: root.queueCursor
                                visibleStart: queueContent.firstVisibleIndex
                                visibleEnd: queueContent.lastVisibleIndex
                                markIndex: row.index
                                markSongId: row.songId
                                markStarred: row.starred
                                searchQuery: queueContent.searchQuery
                                searchCurrent: index === queueContent.searchCurrentRow
                            }

                            HoverHandler {
                                id: rowHover
                                cursorShape: Qt.PointingHandCursor
                            }

                            TapHandler {
                                acceptedModifiers: Qt.ShiftModifier
                                onTapped: {
                                    queueContent.extendSelectionTo(row.index);
                                    root.queueCursor = queueContent.currentIndex;
                                }
                            }
                            TapHandler {
                                acceptedModifiers: Qt.NoModifier
                                onTapped: {
                                    root.queueCursor = row.index;
                                    Backend.queueJump(row.index);
                                }
                            }
                        }
                    }
                }
            }

            FadeText {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: queueItems.count === 0
                text: "Queue is empty"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.height)
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }
    }

    // The queue model stays live while the drawer is closed, so a later
    // Ctrl-Q opens on current rows and the remembered highlight.
    ListModel {
        id: queueItems
    }

    // ARC-UI-1: the recycled Alt verb set; queue rows are tracks.
    QuickActions {
        id: verbs
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "queue")
                root.applyQueue(payload);
        }
        // A fetch finished: re-read the queue's own window (forced).
        function onRowReload() {
            root.requestMarkWindow(true);
        }
        // A favourite landed (per row, no payload): patch every matching
        // queue row in place. The cursor stays where the user left it.
        function onFavoriteChanged(kind, id, on) {
            if (kind !== "song")
                return;
            for (var i = 0; i < queueItems.count; ++i) {
                if (queueItems.get(i).songId === id)
                    queueItems.setProperty(i, "starred", on === true);
            }
        }
        // A skip/advance is deduped backend-side: the index property carries
        // the move — adopt it and follow exactly like a fresh payload would.
        function onQueue_index_changed() {
            if (root.queueIndex === Backend.queueIndex)
                return;
            root.queueIndex = Backend.queueIndex;
            root.recallQueueCursor(true);
        }
    }
}
