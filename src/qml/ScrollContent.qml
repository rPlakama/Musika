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
import "ScrollProfiles.js" as ScrollProfiles
import "Theme.js" as Theme

// The one scroll surface (the "ScrollModal").
//
// Qt owns the geometry: ListView/GridView `currentIndex`, `contentHeight`,
// `contentY`, and `positionViewAtIndex` are authoritative. The follow keeps
// the Neovim `scrolloff=999` feel by asking Qt for a centred position; Qt
// clamps the target at the content ends, so the first and last screens pin to
// the edge without padding. Only the glide itself is custom (Qt has no
// animated `positionViewAtIndex`): a short OutCubic `contentY` animation.
//
// Hosts never write `contentY` or call `positionViewAtIndex` themselves.
Item {
    id: root

    // ── Context profile ──────────────────────────────────────────────────

    /// Context name selecting the metrics/flags defaults (SCROLLMODAL).
    property string context: ""
    readonly property var profile: ScrollProfiles.get(root.context)
    /// Context-only keyboard permissions (e.g. queue reorder).
    property var flags: root.profile.flags !== undefined
        ? root.profile.flags : ({})

    function allows(name) {
        return root.flags[name] === true
    }

    // ── Inputs ───────────────────────────────────────────────────────────

    property var model: null
    property Component rowDelegate: null
    property Component cellDelegate: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: root.profile.bleedRatio !== undefined
        ? root.width * root.profile.bleedRatio : 0

    /// ARC-UI-3: supported layouts in toggle order ("list", "grid").
    property var layouts: ["list"]
    property string layout: "list"
    /// Grid geometry: columns per row and the cell height (0 = square).
    property int columns: root.profile.columns !== undefined
        ? root.profile.columns : 1
    property real cellHeight: root.profile.cellAspect !== undefined
        ? root.cellWidth * root.profile.cellAspect
          + root.height * (root.profile.cellExtra || 0)
        : 0
    readonly property real cellWidth: Math.max(1, root.width / Math.max(1, root.columns))

    /// Row sizing: nominal fraction of the reference height (0 = delegates
    /// own their heights, e.g. lyrics).
    property real rowRatio: root.profile.rowRatio !== undefined
        ? root.profile.rowRatio : 0
    /// The host's stable sizing reference (panel body room, page height…),
    /// never the body's own height (that would feed the row unit back into
    /// the layout).
    property real rowRefHeight: 0
    property real rowHeightScale: root.profile.rowHeightScale !== undefined
        ? root.profile.rowHeightScale : 1.0
    property real rowSpacing: 0
    /// Extra rows kept alive above and below the viewport, so a continuous
    /// scroll never has to create a delegate at the edge.
    property int renderOverscanRows: 10
    readonly property real renderOverscan: {
        var stride = root._stride(root.view);
        return stride > 0 ? stride * root.renderOverscanRows : 0;
    }
    property real minRowHeight: 18
    /// Kept for host compatibility; the extent is Qt's own now.
    property real contentHeightHint: 0
    property bool recenterOnWidth: root.layout === "grid"
    /// Bottom-align a short uniform list instead of leaving its rows at
    /// the top of an otherwise empty viewport (album pages).
    property bool bottomAlignShort: false
    /// Optional content rendered in the list header, just above the first
    /// row (a section title, for example).
    property Component headerContent: null

    /// Follow duration (ms); 0 snaps. `reduceMotion` also snaps.
    property int followMs: Theme.motion.scroll
    /// One-shot duration for the next glide (0 = `followMs`).
    property int glideOverrideMs: 0
    /// Kept for host compatibility; the follow is always centred now.
    property bool unlimitedScrolloff: false

    // ── Search / memory / selection inputs ───────────────────────────────

    property var searchFields: ["title"]
    property string searchQuery: ""
    property int searchCurrentRow: -1
    property string memoryKey: ""
    property int memoryScope: -1
    property bool selectable: false
    property bool resetPending: false

    // ── Row metric ───────────────────────────────────────────────────────

    /// A list with a uniform row unit (grids use their cell, variable-height
    /// lists such as lyrics have none).
    readonly property bool adaptiveRows:
        root.layout !== "grid" && root.rowRatio > 0

    function adaptiveRowFor(viewHeight) {
        if (root.rowRatio <= 0)
            return 0;
        return Math.max(root.minRowHeight,
            viewHeight * root.rowRatio * root.rowHeightScale);
    }

    readonly property real adaptiveRowHeight: root.adaptiveRowFor(
        root.rowRefHeight > 0 ? root.rowRefHeight : root.height)
    readonly property real rowStride:
        root.adaptiveRows ? root.adaptiveRowHeight + root.rowSpacing : 0

    /// Native ListView header that fills the shortfall when a uniform list
    /// does not fill the viewport, pushing its rows to the bottom. The gap
    /// must come from row math (not contentHeight), or the header would feed
    /// its own height back into the calculation.
    readonly property real _shortContentGap: {
        var v = root.view;
        if (!root.bottomAlignShort || !v || v.count === 0 || v.height <= 0)
            return 0;
        if (root.layout === "grid") {
            var cell = root.cellHeight > 0 ? root.cellHeight : root.cellWidth;
            var rows = Math.ceil(v.count / Math.max(1, root.columns));
            return Math.max(0, v.height - rows * cell);
        }
        if (root.rowStride <= 0)
            return 0;
        var rowsHeight = v.count * root.rowStride - root.rowSpacing;
        return Math.max(0, v.height - rowsHeight);
    }

    // ── Cursor ───────────────────────────────────────────────────────────

    property int currentIndex: -1
    onCurrentIndexChanged: root._cursorMoved()
    signal itemActivated(int index)

    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: {
        root.stopGlide();
        viewLandTimer.restart();
    }
    onLayoutChanged: {
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    // ── Geometry helpers ─────────────────────────────────────────────────

    function _stride(v) {
        if (root.layout === "grid")
            return root.cellHeight > 0 ? root.cellHeight : root.cellWidth;
        if (root.rowStride > 0)
            return root.rowStride;
        // Variable-height rows (lyrics) are measured, not derived.
        var i = root.currentIndex >= 0 ? root.currentIndex : 0;
        var item = v ? v.itemAtIndex(i) : null;
        if (item && item.height > 0)
            return item.height + root.rowSpacing;
        return 0;
    }

    /// Full rows in the viewport.
    function visibleRows() {
        var v = root.view;
        var stride = root._stride(v);
        if (!v || stride <= 0 || v.height <= 0)
            return 1;
        return Math.max(1, Math.floor(v.height / stride + 1e-6));
    }

    readonly property int visibleRowCount: root.visibleRows()

    /// First and last *item* indices in the rendered band (viewport plus
    /// overscan), used for marks and mark requests.
    readonly property var _visibleBounds: {
        var v = root.view;
        if (!v || v.count === 0 || v.height <= 0)
            return { first: -1, last: -1 };
        var overscan = root.renderOverscanRows;
        if (root.layout === "grid") {
            var cell = root.cellHeight > 0 ? root.cellHeight : root.cellWidth;
            var stride = Math.max(1, cell);
            var rows = Math.ceil(v.count / Math.max(1, root.columns));
            var firstRow = Math.max(0, Math.floor(v.contentY / stride) - overscan);
            var lastRow = Math.min(rows - 1,
                Math.ceil((v.contentY + v.height) / stride) - 1 + overscan);
            var cols = Math.max(1, root.columns);
            return {
                first: Math.min(v.count - 1, firstRow * cols),
                last: Math.min(v.count - 1, (lastRow + 1) * cols - 1)
            };
        }
        var first = v.indexAt(v.width * 0.5, v.contentY);
        var last = v.indexAt(v.width * 0.5, v.contentY + v.height - 1);
        if (first < 0)
            first = 0;
        if (last < 0)
            last = v.count - 1;
        return {
            first: Math.max(0, first - overscan),
            last: Math.min(v.count - 1, last + overscan)
        };
    }
    readonly property int firstVisibleIndex: root._visibleBounds.first
    readonly property int lastVisibleIndex: root._visibleBounds.last

    /// Let Qt compute the centred (and edge-clamped) target, then restore the
    /// live viewport so the glide can animate from where it actually is.
    function _target(v, index) {
        if (!v || index < 0 || index >= v.count)
            return null;
        var old = v.contentY;
        v.positionViewAtIndex(index, ListView.Center);
        var target = v.contentY;
        v.contentY = old;
        return target;
    }

    /// The one follow: native target, short contentY glide.
    function _follow(v, index, instant) {
        if (!v || v.count === 0 || index < 0 || index >= v.count)
            return;
        root._manualScroll = false;
        var override = root.glideOverrideMs;
        root.glideOverrideMs = 0;
        glide.stop();

        // Instant landings are one native positioning call; the target/probe
        // dance is only for the animated glide.
        if (instant || Theme.reduceMotion || root.followMs <= 0) {
            if (index === 0)
                v.positionViewAtIndex(0, ListView.Beginning);
            else if (index === v.count - 1)
                v.positionViewAtIndex(index, ListView.End);
            else
                v.positionViewAtIndex(index, ListView.Center);
            root._settle();
            return;
        }

        var target = root._target(v, index);
        if (target === null)
            return;
        if (Math.abs(target - v.contentY) < 0.5) {
            v.contentY = target;
            root._settle();
            return;
        }
        glide.from = v.contentY;
        glide.to = target;
        glide.duration = override > 0 ? override : root.followMs;
        glide.start();
    }

    function _settle() {
        if (root.memoryKey)
            root._scheduleRemember();
    }

    function stopGlide() {
        glide.stop();
        root.glideOverrideMs = 0;
    }

    /// Re-run the follow for the current cursor over `duration` ms.
    function recenter(duration) {
        if (root.currentIndex < 0 || !root.view)
            return;
        root._manualScroll = false;
        root.glideOverrideMs = Math.max(0, duration);
        root._follow(root.view, root.currentIndex, false);
    }

    /// Snap the cursor to its native centred position now.
    function recenterNow() {
        if (root.currentIndex < 0 || !root.view)
            return;
        root._manualScroll = false;
        root.glideOverrideMs = 0;
        root._follow(root.view, root.currentIndex, true);
    }

    function _cursorMoved() {
        if (root._landing)
            return;
        root._manualScroll = false;
        var v = root.view;
        if (!v)
            return;
        if (root.currentIndex < 0) {
            if (v.currentIndex !== -1)
                v.currentIndex = -1;
            return;
        }
        if (v.count === 0)
            return;
        var index = Math.min(v.count - 1, root.currentIndex);
        if (index !== root.currentIndex) {
            root.currentIndex = index;
            return;
        }
        if (v.currentIndex !== index)
            v.currentIndex = index;
        root._follow(v, index, false);
    }

    /// Instant landing (semantic jumps: gg/G, search, restores, reset).
    function land(index) {
        root._land(index);
    }

    function _land(index) {
        var v = root.view;
        if (!v || v.count === 0)
            return;
        index = Math.max(0, Math.min(index, v.count - 1));
        root._manualScroll = false;
        // Suppress the glide `_cursorMoved` would start; a landing is one
        // native positioning call.
        root._landing = true;
        if (root.currentIndex !== index)
            root.currentIndex = index;
        if (v.currentIndex !== index)
            v.currentIndex = index;
        root._landing = false;
        root._follow(v, index, true);
    }

    /// A surface returning to screen calls this: land on the cursor.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing jump: force a follow even when the index already
    /// matches (a manual scroll may have moved the viewport).
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        if (root.count > 0)
            i = Math.max(0, Math.min(i, root.count - 1));
        root._manualScroll = false;
        root._pendingFollow = i;
        root._landing = true;
        if (root.currentIndex !== i)
            root.currentIndex = i;
        var v = root.view;
        if (!v || v.count === 0) {
            root._landing = false;
            return;
        }
        if (v.currentIndex !== i)
            v.currentIndex = i;
        root._landing = false;
        root._pendingFollow = -1;
        root._follow(v, i, false);
    }

    property int _pendingFollow: -1
    property bool _countRepairScheduled: false

    function _scheduleCountRepair() {
        if (root._countRepairScheduled)
            return;
        root._countRepairScheduled = true;
        countRepairTimer.restart();
    }

    function _repairCount() {
        root._countRepairScheduled = false;
        var v = root.view;
        if (!v)
            return;
        if (v.count === 0) {
            if (root.currentIndex !== -1)
                root.currentIndex = -1;
            return;
        }
        if (root.currentIndex >= v.count) {
            root.land(v.count - 1);
            return;
        }
        if (root.currentIndex < 0)
            return;
        if (v.currentIndex !== root.currentIndex)
            v.currentIndex = root.currentIndex;
        if (root._pendingFollow >= 0 && root._pendingFollow === root.currentIndex
                && v.count > root._pendingFollow) {
            var i = root._pendingFollow;
            root._pendingFollow = -1;
            root._follow(v, i, false);
        }
        if (root.memoryKey)
            root.remember();
    }

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        glide.stop();
        if (root.view)
            root.view.positionViewAtBeginning();
    }

    /// `gg`/`G`: instant landing. A count turns it into a line jump.
    function edgeJump(last, count) {
        if (root.count === 0)
            return;
        var n = count === undefined ? 1 : Math.max(1, count);
        if (n > 1) {
            root.land(Math.min(n, root.count) - 1);
            return;
        }
        root.land(last ? root.count - 1 : 0);
    }

    /// Shared cursor step: in grid mode the wall walks by `columns`; in list
    /// mode it is a plain ±1. `count` steps at once, clamped at the edges.
    function moveStep(action, count) {
        if (root.count === 0)
            return false;
        var n = count === undefined ? 1 : Math.max(1, count);
        var cols = root.layout === "grid" ? Math.max(1, root.columns) : 1;
        var column = root.currentIndex % cols;
        var target = root.currentIndex;
        if (action === "move_left") {
            if (column === 0)
                return false;
            target -= Math.min(n, column);
        } else if (action === "move_right") {
            if (column === cols - 1 || root.currentIndex + 1 >= root.count)
                return false;
            target += Math.min(n, cols - 1 - column, root.count - 1 - root.currentIndex);
        } else if (action === "move_up") {
            if (root.currentIndex - cols * n < 0) {
                if (n === 1)
                    return false;
                target = column;
            } else {
                target -= cols * n;
            }
        } else if (action === "move_down") {
            if (root.currentIndex + cols * n >= root.count) {
                if (n === 1)
                    return false;
                target = root.count - 1;
            } else {
                target += cols * n;
            }
        } else {
            return false;
        }
        if (target === root.currentIndex)
            return false;
        root.currentIndex = target;
        return true;
    }

    /// `Ctrl-D`/`Ctrl-U`: move by `fraction` of the viewport, in rows.
    function pageJump(down, fraction) {
        if (root.count === 0 || !root.view)
            return false;
        var rowH = root.layout === "grid"
            ? (root.cellHeight > 0 ? root.cellHeight : root.cellWidth)
            : root.adaptiveRowHeight;
        if (rowH <= 0)
            rowH = root._stride(root.view);
        if (rowH <= 0)
            return false;
        var rows = Math.max(1, Math.floor(
            root.view.height * (fraction === undefined ? 0.5 : fraction) / rowH));
        return root.moveStep(down ? "move_down" : "move_up", rows);
    }

    function activate() {
        if (root.currentIndex >= 0)
            root.itemActivated(root.currentIndex);
    }

    // ── Memory (UI-54) ───────────────────────────────────────────────────

    function restore() {
        root.applySavedLayout();
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        if (root.count > 0)
            index = Math.max(0, Math.min(index, root.count - 1));
        root._land(index);
        var recalled = scroll.recall(root.memoryKey, root.view);
        return recalled.ok || root.count === 0;
    }

    function remember(scope) {
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view,
                scope === undefined ? root.memoryScope : scope, root.layout,
                root.currentIndex);
    }

    property bool _rememberScheduled: false
    function _scheduleRemember() {
        if (!root.memoryKey || root._rememberScheduled)
            return;
        root._rememberScheduled = true;
        rememberTimer.restart();
    }
    function _flushRemember() {
        root._rememberScheduled = false;
        if (root.memoryKey)
            root.remember();
    }
    Component.onDestruction: root._flushRemember()

    // ── Layout helpers (UI-58 / ARC-UI-3) ────────────────────────────────

    function toggleLayout() {
        if (root.layouts.length < 2)
            return;
        var i = root.layouts.indexOf(root.layout);
        root.layout = root.layouts[(i + 1) % root.layouts.length];
    }

    function applySavedLayout() {
        var saved = scroll.savedLayout(root.memoryKey);
        if (saved.length > 0 && root.layouts.indexOf(saved) >= 0)
            root.layout = saved;
    }

    // ── Resize ───────────────────────────────────────────────────────────

    property bool _manualScroll: false
    property bool _landing: false
    property bool _resizeScheduled: false

    /// One deferred pass per resize batch, off the compositor event.
    function _scheduleResizeRecenter() {
        if (!root.view || root._resizeScheduled)
            return;
        root._resizeScheduled = true;
        resizeTimer.restart();
    }

    function _resizeSettle() {
        root._resizeScheduled = false;
        var v = root.view;
        if (!v)
            return;
        root.stopGlide();
        if (root.currentIndex >= 0 && root.currentIndex < v.count) {
            if (v.currentIndex !== root.currentIndex)
                v.currentIndex = root.currentIndex;
            root._follow(v, root.currentIndex, true);
        }
    }

    // Timers, not Qt.callLater: a Loader can destroy this ScrollContent
    // between scheduling and delivery, and a destroyed object must not try to
    // evaluate a queued function in its dead context.
    Timer {
        id: resizeTimer
        interval: 0
        repeat: false
        onTriggered: root._resizeSettle()
    }
    Timer {
        id: viewLandTimer
        interval: 0
        repeat: false
        onTriggered: root.landCursor()
    }
    Timer {
        id: countRepairTimer
        interval: 0
        repeat: false
        onTriggered: root._repairCount()
    }
    Timer {
        id: rememberTimer
        interval: 0
        repeat: false
        onTriggered: root._flushRemember()
    }

    Component.onCompleted: root.applySavedLayout()
    onMemoryKeyChanged: root.applySavedLayout()
    onWidthChanged: if (root.recenterOnWidth) root._scheduleResizeRecenter()
    onHeightChanged: root._scheduleResizeRecenter()
    onAdaptiveRowHeightChanged: root._scheduleResizeRecenter()
    onCellHeightChanged: root._scheduleResizeRecenter()
    onColumnsChanged: root._scheduleResizeRecenter()

    // ── Selection (VI-4 / ARC-I-4) ───────────────────────────────────────

    signal selectionChanged

    RowState {
        id: rowState
        model: root.model
        cursor: root.currentIndex
        selectable: root.selectable
    }
    Connections {
        target: rowState
        function onSelectionChanged() { root.selectionChanged(); }
    }

    readonly property bool hasSelection: rowState.hasSelection
    readonly property int selectionCount: rowState.selectionCount

    function toggleSelection() { rowState.toggle(); }
    function selectAllRows() { rowState.selectAllRows(); }
    function extendSelection(action) {
        if (!rowState.extendTo(root.currentIndex))
            return false;
        return root.moveStep(action, 1);
    }
    function extendSelectionTo(index) {
        if (index < 0 || index >= root.count
                || !rowState.extendTo(root.currentIndex))
            return false;
        root.currentIndex = index;
        return true;
    }
    function clearSelection() { rowState.clear(); }
    function selected(index) { return rowState.selected(index); }
    function selectionIds(field) { return rowState.ids(field); }
    function selectionIndices() { return rowState.indices(); }
    function selectionStarred(field) { return rowState.starred(field); }
    function markStarred(on) { rowState.markStarred(on); }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    SearchContract {
        id: searchContract
        model: root.model
        view: root.view
        fields: root.searchFields
    }

    function searchRows() { return searchContract.rows(); }
    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.land(row);
    }

    // ── Reset (UI-10) ────────────────────────────────────────────────────

    function resetView() {
        root.resetPending = true;
        glide.stop();
        root._pendingFollow = -1;
        root._manualScroll = false;
        root.currentIndex = -1;
        if (root.view) {
            root.view.currentIndex = -1;
            root.view.positionViewAtBeginning();
        }
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    // ── The views ────────────────────────────────────────────────────────

    Layout.fillWidth: true
    Layout.fillHeight: true

    ScrollMemory {
        id: scroll
    }

    NumberAnimation {
        id: glide
        target: root.view
        property: "contentY"
        easing.type: Easing.OutCubic
        onFinished: root._settle()
    }

    // UI-48: the stage is the clip owner when bleed is on.
    Item {
        id: stage
        x: -root.bleed
        width: root.width + 2 * root.bleed
        height: root.height
        clip: true

        ListView {
            id: list
            anchors.fill: parent
            anchors.leftMargin: root.bleed
            anchors.rightMargin: root.bleed
            clip: root.bleed <= 0
            visible: root.layout !== "grid"
            model: root.layout === "grid" ? null : root.model
            delegate: root.rowDelegate
            spacing: root.rowSpacing
            header: Column {
                width: list.width
                Item {
                    width: parent.width
                    height: Math.max(0,
                        root._shortContentGap - headerLoader.height)
                }
                Loader {
                    id: headerLoader
                    width: parent.width
                    sourceComponent: root.headerContent
                }
            }
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: false
            cacheBuffer: root.renderOverscan
        }

        GridView {
            id: grid
            anchors.fill: parent
            anchors.leftMargin: root.bleed
            anchors.rightMargin: root.bleed
            clip: root.bleed <= 0
            visible: root.layout === "grid"
            model: root.layout === "grid" ? root.model : null
            delegate: root.cellDelegate
            cellWidth: root.cellWidth
            header: Column {
                width: grid.width
                Item {
                    width: parent.width
                    height: Math.max(0,
                        root._shortContentGap - headerLoaderGrid.height)
                }
                Loader {
                    id: headerLoaderGrid
                    width: parent.width
                    sourceComponent: root.headerContent
                }
            }
            cellHeight: root.cellHeight > 0 ? root.cellHeight : root.cellWidth
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: false
            cacheBuffer: root.renderOverscan
        }
    }

    Connections {
        target: root.view
        function onContentYChanged() {
            if (root.memoryKey && !glide.running)
                root._scheduleRemember();
        }
        function onMovementStarted() {
            root.stopGlide();
            root._manualScroll = true;
        }
        function onCurrentIndexChanged() {
            if (target.count === 0)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            if (root.memoryKey)
                root.remember();
        }
        function onCountChanged() {
            if (target.count === 0) {
                if (root.currentIndex !== -1)
                    root.currentIndex = -1;
                return;
            }
            root._scheduleCountRepair();
        }
    }
}
