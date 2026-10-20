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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import QtQuick
import QtQuick.Layouts
import "Theme.js" as Theme

// The one scroll surface (the "ScrollModal").
//
// Inputs are deliberately small: a model (its count is the row amount), the
// viewport's own width/height, and a delegate. Everything else — the row unit,
// how many rows fit, which rows exist, where the cursor sits and where the
// viewport must stop — is solved here.
//
// Follow model (Neovim `scrolloff=999`):
//   * while the cursor is away from both ends the row stays centred;
//   * once the cursor's row reaches the first/last screenful the viewport
//     pins to the content edge, so centring stops exactly when the top or the
//     last row is in view — no padding, no empty gap.
//
// The extent is derived from the row unit (count × row height) instead of
// trusting Qt's lazy `contentHeight` estimate; a realised last row overrides
// it with real geometry. The rendered set is capped to the rows actually on
// screen: `cacheBuffer: 0` adds no prediction band and `reuseItems: false`
// destroys a delegate as soon as it leaves the viewport instead of keeping
// an offscreen pool alive.
//
// The follow is a frame loop: it re-reads the target every frame, absorbs
// geometry drift, and glides with OutCubic. Semantic jumps land instantly.
// Hosts never write `contentY` or call `positionViewAtIndex` themselves.
Item {
    id: root

    // ── Inputs ───────────────────────────────────────────────────────────

    property var model: null
    property Component rowDelegate: null
    property Component cellDelegate: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: 0

    /// ARC-UI-3: supported layouts in toggle order ("list", "grid").
    property var layouts: ["list"]
    property string layout: "list"
    /// Grid geometry: columns per row and the cell height (0 = square).
    property int columns: 1
    property real cellHeight: 0
    readonly property real cellWidth: Math.max(1, root.width / Math.max(1, root.columns))

    /// Row sizing: nominal fraction of the reference height (0 = delegates
    /// own their heights, e.g. lyrics).
    property real rowRatio: 0
    property real rowRefHeight: 0
    /// Built-in row unit for the fullscreen queue. Hosts that need a custom
    /// unit leave this empty and use `rowRatio`/`rowRefHeight` instead.
    property string rowPreset: ""
    readonly property real _rowRatio:
        root.rowPreset === "queue" ? 0.16 : root.rowRatio
    readonly property real rowViewHeight:
        root.rowRefHeight > 0 ? root.rowRefHeight : root.height
    property real rowHeightScale: 1.0
    property real rowSpacing: 0
    /// A transient tiny viewport (the player's panes start at a few pixels
    /// before the split settles) must not turn the rows microscopic: at 1.4 px
    /// the visible band spanned ~130 rows and realised all of them.
    property real minRowHeight: 18
    /// Host-supplied extent (0 = derive it from the row unit).
    property real contentHeightHint: 0
    property bool recenterOnWidth: root.layout === "grid"

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

    // ── Cursor ───────────────────────────────────────────────────────────

    property int currentIndex: -1
    onCurrentIndexChanged: root._cursorMoved()
    signal itemActivated(int index)

    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: {
        root.stopGlide();
        Qt.callLater(root.landCursor);
    }
    onLayoutChanged: {
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    // ── The follow ───────────────────────────────────────────────────────

    FrameAnimation {
        id: glide
        onTriggered: root._tick()
    }

    property real _startY: 0
    property real _elapsed: 0
    property int _duration: 0
    property real _lastTarget: 0
    property int _revealTries: 0

    /// Rows in the viewport: viewport height divided by the row stride. Rows
    /// are fitted to whole counts, so rounding (not ceiling) keeps the
    /// screenful exact and the centring band stable across float error.
    function visibleRows() {
        var v = root.view;
        var stride = root._stride(v);
        if (!v || stride <= 0 || v.height <= 0)
            return 1;
        return Math.max(1, Math.round(v.height / stride));
    }

    /// The screenful as a reactive property (the render/mark cap).
    readonly property int visibleRowCount: root.visibleRows()

    /// First and last *item* indices currently inside the viewport. List rows
    /// are their own index; a grid returns the whole visible cell band. These
    /// are the only rows whose marks are allowed to exist.
    readonly property var _visibleBounds: {
        var v = root.view;
        if (!v || v.count === 0 || v.height <= 0)
            return { first: -1, last: -1 };
        if (root.layout === "grid") {
            var cell = root.cellHeight > 0 ? root.cellHeight : root.cellWidth;
            var stride = Math.max(1, cell);
            var firstRow = Math.max(0, Math.floor(v.contentY / stride));
            var lastRow = Math.max(firstRow,
                Math.ceil((v.contentY + v.height) / stride) - 1);
            var cols = Math.max(1, root.columns);
            return {
                first: Math.min(v.count - 1, firstRow * cols),
                last: Math.min(v.count - 1, (lastRow + 1) * cols - 1)
            };
        }
        var rowStride = root._stride(v);
        if (rowStride <= 0)
            return { first: 0, last: v.count - 1 };
        return {
            first: Math.max(0, Math.floor(v.contentY / rowStride)),
            last: Math.min(v.count - 1,
                Math.ceil((v.contentY + v.height) / rowStride) - 1)
        };
    }
    readonly property int firstVisibleIndex: root._visibleBounds.first
    readonly property int lastVisibleIndex: root._visibleBounds.last

    function _stride(v) {
        if (root.layout === "grid")
            return root.cellHeight > 0 ? root.cellHeight : root.cellWidth;
        var i = root.currentIndex >= 0 ? root.currentIndex : 0;
        var item = v ? v.itemAtIndex(i) : null;
        if (item && item.height > 0)
            return item.height + root.rowSpacing;
        var nominal = root.adaptiveRowHeight;
        return nominal > 0 ? nominal + root.rowSpacing : 0;
    }

    function _rowOf(index) {
        return root.layout === "grid"
            ? Math.floor(index / Math.max(1, root.columns)) : index;
    }

    function _rowCount(v) {
        return root.layout === "grid"
            ? Math.ceil(v.count / Math.max(1, root.columns)) : v.count;
    }

    /// The content extent. Uniform rows are solved from count × row unit; a
    /// realised last row supplies the exact tail.
    function _extent(v) {
        if (!v)
            return 0;
        if (root.contentHeightHint > 0)
            return root.contentHeightHint;
        var last = v.count > 0 ? v.itemAtIndex(v.count - 1) : null;
        if (last && last.height > 0)
            return last.y + last.height;
        var stride = root._stride(v);
        var uniform = root.layout === "grid" || root._rowRatio > 0;
        if (uniform && stride > 0) {
            if (root.layout === "grid")
                return root._rowCount(v) * stride;
            return Math.max(0, v.count * stride - root.rowSpacing);
        }
        return v.contentHeight;
    }

    function _maxY(v) {
        if (!v)
            return 0;
        return Math.max(0, root._extent(v) - v.height);
    }

    /// Where `contentY` must be for `index`. `null` when the row cannot be
    /// measured yet and no row unit exists.
    function _target(v, index) {
        if (!v || index < 0 || index >= v.count)
            return null;
        var maxY = root._maxY(v);
        var rows = root._rowCount(v);
        var row = root._rowOf(index);
        var half = root.visibleRows() * 0.5;
        // Centring only in the middle band. The edge rows pin the viewport,
        // exactly like `scrolloff=999` at the start/end of a file.
        if (row + 1 <= half)
            return 0;
        if (row >= rows - half)
            return maxY;
        var item = v.itemAtIndex(index);
        var center;
        if (item && item.height > 0) {
            center = item.y + item.height * 0.5 - v.height * 0.5;
        } else {
            var stride = root._stride(v);
            if (stride <= 0)
                return null;
            center = (row + 0.5) * stride - v.height * 0.5;
        }
        return Math.max(0, Math.min(maxY, center));
    }

    function _clampY(v) {
        if (!v)
            return;
        var maxY = root._maxY(v);
        if (v.contentY < 0 || v.contentY > maxY)
            v.contentY = Math.max(0, Math.min(maxY, v.contentY));
    }

    /// Bring an unrealised row into the viewport without Qt's async
    /// Contain/Center offsets: anchor it at the top, then centre it.
    function _reveal(v, index) {
        if (!v || index < 0 || index >= v.count)
            return;
        v.positionViewAtIndex(index, ListView.Beginning);
        root._clampY(v);
    }

    function _follow(v, index, instant) {
        if (!v || v.count === 0 || index < 0 || index >= v.count
                || root.isLanding)
            return;
        var item = v.itemAtIndex(index);
        if (!item) {
            // A row that never realises (zero-height view, a bad delegate)
            // must not spin the callLater loop forever.
            if (root._revealTries >= 4) {
                root._clampY(v);
                return;
            }
            root._revealTries++;
            root._reveal(v, index);
            Qt.callLater(function () {
                if (root.currentIndex === index && root.view)
                    root._follow(root.view, index, instant);
            });
            return;
        }
        root._revealTries = 0;
        var target = root._target(v, index);
        if (target === null)
            return;
        var override = root.glideOverrideMs;
        root.glideOverrideMs = 0;
        glide.stop();
        if (instant || Theme.reduceMotion || root.followMs <= 0
                || Math.abs(target - v.contentY) < 0.5) {
            v.contentY = target;
            root._settle();
            return;
        }
        root._startY = v.contentY;
        root._lastTarget = target;
        root._elapsed = 0;
        root._duration = override > 0 ? override : root.followMs;
        glide.start();
    }

    function _tick() {
        var v = root.view;
        if (!v || root.currentIndex < 0 || root.currentIndex >= v.count
                || root.isLanding) {
            glide.stop();
            return;
        }
        var target = root._target(v, root.currentIndex);
        if (target === null) {
            glide.stop();
            return;
        }
        // Dynamic rows re-measure under the cursor; absorb the drift into the
        // start point so the interpolation never jumps.
        var drift = target - root._lastTarget;
        if (Math.abs(drift) > 0.5) {
            root._startY += drift;
            root._lastTarget = target;
        }
        root._elapsed += glide.frameTime * 1000;
        var t = root._duration > 0
            ? Math.min(1, root._elapsed / root._duration) : 1;
        var eased = 1 - Math.pow(1 - t, 3);
        v.contentY = root._startY + (target - root._startY) * eased;
        if (t >= 1 || Math.abs(v.contentY - target) < 0.5) {
            v.contentY = target;
            glide.stop();
            root.glideOverrideMs = 0;
            root._settle();
        }
    }

    function _settle() {
        var v = root.view;
        if (!v)
            return;
        // Safety net for a row unit that disagrees with real geometry: never
        // leave the cursor fully outside the viewport.
        var item = root.currentIndex >= 0
            ? v.itemAtIndex(root.currentIndex) : null;
        if (item && item.height > 0) {
            var y = v.contentY;
            if (item.y < y - 0.5)
                y = item.y;
            else if (item.y + item.height > y + v.height + 0.5)
                y = item.y + item.height - v.height;
            if (Math.abs(y - v.contentY) > 1)
                v.contentY = y;
        }
        root._clampY(v);
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
        root.glideOverrideMs = Math.max(0, duration);
        root._follow(root.view, root.currentIndex, false);
    }

    /// Snap the cursor to the centre (or its edge pin) now.
    function recenterNow() {
        if (root.currentIndex < 0 || !root.view)
            return;
        root.glideOverrideMs = 0;
        root._follow(root.view, root.currentIndex, true);
    }

    function _cursorMoved() {
        if (root.isLanding)
            return;
        var v = root.view;
        if (root.currentIndex < 0) {
            if (v && v.currentIndex !== -1)
                v.currentIndex = -1;
            return;
        }
        if (!v || v.count === 0)
            return;
        // A host can write past a shrunken payload; keep both copies equal.
        var index = Math.min(v.count - 1, root.currentIndex);
        if (index !== root.currentIndex) {
            root.currentIndex = index;
            return;
        }
        if (v.currentIndex !== index)
            v.currentIndex = index;
        root._follow(v, index, false);
    }

    // ── Landings ─────────────────────────────────────────────────────────

    property bool _instantFollow: false
    readonly property bool isLanding: root._instantFollow
    property int _pendingFollow: -1
    property bool _countRepairScheduled: false
    property bool _modelReset: false

    /// Instant landing (semantic jumps: gg/G, search, restores, reset).
    function land(index) {
        root._land(index, true);
    }

    function _land(index, withSettle) {
        if (index < 0)
            return;
        var v = root.view;
        if (v && v.count > 0)
            index = Math.max(0, Math.min(index, v.count - 1));
        glide.stop();
        root._modelReset = false;
        root._instantFollow = true;
        root._revealTries = 0;
        root.currentIndex = index;
        var unrealised = v && v.count > 0 && !v.itemAtIndex(index);
        if (v && v.count > 0) {
            v.currentIndex = index;
            if (unrealised)
                root._reveal(v, index);
            var target = root._target(v, index);
            if (target !== null)
                v.contentY = target;
            root._clampY(v);
        }
        Qt.callLater(function () {
            root._instantFollow = false;
            var w = root.view;
            if (!w || w.count === 0 || root.currentIndex !== index)
                return;
            if (withSettle && unrealised) {
                // The anchor only got the row on screen; centre it now that
                // it exists (a memory restore owns its own offset).
                root._follow(w, index, true);
            } else if (withSettle) {
                root._settle();
            }
        });
    }

    /// A surface returning to screen calls this: land on the cursor.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing jump: force a follow even when the index already
    /// matches (a manual scroll or a resize may have moved the viewport).
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        if (root.count > 0)
            i = Math.max(0, Math.min(i, root.count - 1));
        root._pendingFollow = i;
        root.currentIndex = i;
        var v = root.view;
        if (!v || v.count === 0)
            return; // the count repair finishes the follow
        if (!v.itemAtIndex(i)) {
            root._land(i, false);
            return;
        }
        root._pendingFollow = -1;
        root._follow(v, i, false);
    }

    function _scheduleCountRepair() {
        if (root._countRepairScheduled)
            return;
        root._countRepairScheduled = true;
        Qt.callLater(root._repairCount);
    }

    function _repairCount() {
        root._countRepairScheduled = false;
        var v = root.view;
        if (!v || v.count === 0 || root.resetPending)
            return;
        if (root.isLanding) {
            root._scheduleCountRepair();
            return;
        }
        var rebuilt = root._modelReset;
        root._modelReset = false;
        if (rebuilt && root.currentIndex >= 0)
            root._land(root.currentIndex, false);
        else if (root.currentIndex < 0) {
            if (scroll.savedIndex(root.memoryKey) < 0)
                root._land(0, false);
        } else if (root.currentIndex > v.count - 1) {
            root._land(v.count - 1, false);
        } else if (v.currentIndex !== root.currentIndex) {
            root._land(root.currentIndex, false);
        }
        if (root.memoryKey)
            root.remember();
        if (root._pendingFollow >= v.count)
            root._pendingFollow = -1;
        if (root._pendingFollow >= 0 && root._pendingFollow === root.currentIndex
                && v.count > root._pendingFollow) {
            var i = root._pendingFollow;
            root._pendingFollow = -1;
            Qt.callLater(function () {
                if (root.currentIndex === i && root.view && root.view.count > 0)
                    root._follow(root.view, i, false);
            });
        }
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
        root._land(index, false);
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
        Qt.callLater(root._flushRemember);
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

    property real _rowBase: 0

    /// Snap `row` so `viewHeight` holds a whole number of rows plus their
    /// gaps. This is the last line of defence against a sliced row.
    function _fitRows(viewHeight, row) {
        if (viewHeight <= 0 || row <= 0)
            return row;
        var gap = root.rowSpacing;
        var rows = Math.max(1, Math.ceil(
            (viewHeight + gap) / (row + gap) - 1e-6));
        return Math.max(0, (viewHeight - (rows - 1) * gap) / rows);
    }

    function adaptiveRowFor(viewHeight) {
        return root._fitRows(viewHeight,
            Theme.adaptiveRow(viewHeight, root._rowRatio, root._rowBase)
            * root.rowHeightScale);
    }

    /// Delegate height: snapped to this body's real viewport (not only the
    /// host's reference), so no half row is ever left on screen.
    readonly property real adaptiveRowHeight:
        Math.max(root.minRowHeight,
            root._fitRows(root.height, root.adaptiveRowFor(root.rowViewHeight)))

    function captureRowBase() {
        if (root._rowBase <= 0 && root._rowRatio > 0 && root.rowViewHeight > 0)
            root._rowBase = root.rowViewHeight * root._rowRatio;
    }

    Component.onCompleted: {
        root.captureRowBase();
        root.applySavedLayout();
    }
    onHeightChanged: {
        if (root.rowRefHeight <= 0)
            root.captureRowBase();
        root._scheduleResizeRecenter();
    }
    onRowRefHeightChanged: root.captureRowBase()
    onMemoryKeyChanged: root.applySavedLayout()
    onWidthChanged: if (root.recenterOnWidth) root._scheduleResizeRecenter()
    onAdaptiveRowHeightChanged: root._scheduleResizeRecenter()
    onCellHeightChanged: root._scheduleResizeRecenter()
    onColumnsChanged: root._scheduleResizeRecenter()

    Timer {
        id: resizeTimer
        interval: 100
        repeat: false
        onTriggered: if (root.currentIndex >= 0) root.recenterNow()
    }

    function _scheduleResizeRecenter() {
        if (root.currentIndex < 0 || !root.view)
            return;
        // A resize invalidates the current target; snap once the geometry
        // settles instead of gliding toward stale row heights.
        root._revealTries = 0;
        root.stopGlide();
        root.recenterNow();
        resizeTimer.restart();
    }

    // ── Row state (VI-4 / ARC-I-4) ───────────────────────────────────────

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
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: false
            cacheBuffer: 0
            displayMarginBeginning: 0
            displayMarginEnd: 0
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
            cellHeight: root.cellHeight > 0 ? root.cellHeight : root.cellWidth
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: false
            cacheBuffer: 0
            displayMarginBeginning: 0
            displayMarginEnd: 0
        }
    }

    Connections {
        target: root.view
        function onContentYChanged() {
            if (root.memoryKey && !glide.running && !root.isLanding)
                root._scheduleRemember();
        }
        function onMovementStarted() {
            root.stopGlide();
        }
        function onContentHeightChanged() {
            if (root.isLanding)
                return;
            // While a glide runs the target absorbs the shift; clamping here
            // would fight the interpolation.
            if (!glide.running)
                root._clampY(target);
        }
        function onCurrentIndexChanged() {
            if (target.count === 0 || root.isLanding)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            if (root.memoryKey)
                root.remember();
        }
        function onCountChanged() {
            if (target.count === 0) {
                root._modelReset = true;
                return;
            }
            root._scheduleCountRepair();
        }
    }
}
