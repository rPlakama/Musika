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

// The shared scroll body.
//
// The model is a viewport (a Rectangle of variable width/height) holding rows
// of variable height. The cursor is the single source of truth; the viewport
// follows it by keeping the cursor row in the middle. Half a viewport of empty
// space sits above row 0 and below the last row, so even the ends can sit in
// the middle (Neovim `scrolloff=999`: the gap is allowed to happen).
//
// The follow is a frame loop, not a fixed animation: every frame re-reads the
// live delegate and re-clamps against the current extent, so a resize, a late
// delegate or a growing `contentHeight` estimate retargets the motion instead
// of leaving it short.
//
// Semantic jumps (`gg`/`G`, search, restore, reset) land instantly; ordinary
// cursor moves glide over `followMs`. Hosts never write `contentY` or call
// `positionViewAtIndex` themselves — this body is the only writer.
Item {
    id: root

    // ── Model & roles ────────────────────────────────────────────────────

    property var model: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: 0
    property Component rowDelegate: null
    /// ARC-UI-3: the grid cell delegate (cover + caption, like ArtistEntry).
    property Component cellDelegate: null

    /// ARC-UI-2 M1: the model roles `/` searches, in paint order.
    property var searchFields: ["title"]
    /// ARC-UI-2 M2: the live `/` session.
    property string searchQuery: ""
    property int searchCurrentRow: -1

    // ── Layout (list ⇄ grid) ─────────────────────────────────────────────

    /// ARC-UI-3: supported layouts in toggle order ("list", "grid").
    property var layouts: ["list"]
    property string layout: "list"
    /// Grid geometry: columns per row and the cell height (0 = square).
    property int columns: 1
    property real cellHeight: 0
    readonly property real cellWidth: Math.max(1, root.width / Math.max(1, root.columns))

    /// ARC-UI-3: `Shift-C` on a host that declares both layouts.
    function toggleLayout() {
        if (root.layouts.length < 2)
            return;
        var i = root.layouts.indexOf(root.layout);
        root.layout = root.layouts[(i + 1) % root.layouts.length];
    }

    /// ARC-UI-3: reopen the body the user left (per `memoryKey`).
    function applySavedLayout() {
        var saved = scroll.savedLayout(root.memoryKey);
        if (saved.length > 0 && root.layouts.indexOf(saved) >= 0)
            root.layout = saved;
    }

    // ── Cursor ───────────────────────────────────────────────────────────

    /// The cursor: the single source of truth. Hosts write it; the live view
    /// mirrors it.
    property int currentIndex: -1
    onCurrentIndexChanged: {
        // A landing owns the view: it positions and mirrors the index itself;
        // starting a glide here would fight it.
        if (root.isLanding)
            return;
        var v = root.view;
        if (v && v.count > 0 && v.currentIndex !== root.currentIndex) {
            v.currentIndex = root.currentIndex;
            root._glideTo(v, root.currentIndex, false);
        }
    }

    /// The active view (itemAtIndex, positionViewAtIndex, …).
    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: Qt.callLater(root.landCursor)
    onLayoutChanged: {
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    // ── Follow ───────────────────────────────────────────────────────────

    /// Follow duration (ms); 0 snaps. `reduceMotion` also snaps.
    property int followMs: Theme.motion.scroll
    /// One-shot duration for the next glide (0 = `followMs`).
    property int glideOverrideMs: 0

    /// Half a viewport of empty space at each end, so the first/last row can
    /// sit in the middle. The list carries it as header/footer; the grid uses
    /// the Flickable margins (GridView has no header).
    readonly property real _edgeMargin: root.height * 0.5

    /// Kept for host compatibility: every surface now uses the end spacers.
    property bool unlimitedScrolloff: false

    FrameAnimation {
        id: centerGlide
        onTriggered: root._followTick()
    }

    property real _followStartY: 0
    property real _followElapsed: 0
    property int _followDuration: 0

    /// The scroll range of the viewpor. The half-viewport end spacers let
    /// `contentY` run from `-margin` (end gap on screen) to the last row's
    /// centre; a list folds both spacers into `contentHeight`, a grid keeps
    /// them in the Flickable margins, hence the `+2m` below.
    function _bounds(v) {
        var m = root._edgeMargin;
        var extent = root.contentExtent(v) + (root.layout === "grid" ? 2 * m : 0);
        var lo = -m;
        return { lo: lo, hi: Math.max(lo, extent - v.height - m) };
    }

    /// The centre target, clamped to the scroll range. `null` when the
    /// cursor row is not realised yet.
    function _centerTarget(v, index) {
        var item = v ? v.itemAtIndex(index) : null;
        if (!item)
            return null;
        var b = root._bounds(v);
        return Math.max(b.lo, Math.min(b.hi,
            item.y + item.height * 0.5 - v.height * 0.5));
    }

    /// Keep `contentY` inside the scroll range.
    function _clampY(v) {
        if (!v)
            return;
        var b = root._bounds(v);
        if (v.contentY < b.lo || v.contentY > b.hi)
            v.contentY = Math.max(b.lo, Math.min(b.hi, v.contentY));
    }

    /// The clamp/centre extent: Qt's live `contentHeight` (or the host's
    /// fixed-row hint).
    function contentExtent(v) {
        if (root.contentHeightHint > 0)
            return root.contentHeightHint;
        return v ? v.contentHeight : 0;
    }

    /// Bring an unrealised row into view: `Beginning` anchors it at the top,
    /// then the caller centres it. Avoids Qt's async `Contain`/`Center`
    /// offsets on a scrolled list.
    function _landUnrealised(v, index) {
        v.positionViewAtIndex(index, ListView.Beginning);
        root._clampY(v);
    }

    /// Follow the cursor. `instant` snaps instead of gliding; an unrealised
    /// row falls back to an instant reveal first.
    function _glideTo(v, index, instant) {
        if (!v || v.count === 0 || index < 0 || root.isLanding)
            return;
        centerGlide.stop();
        root._clampY(v);
        var item = v.itemAtIndex(index);
        if (!item) {
            root._landUnrealised(v, index);
            Qt.callLater(function () {
                if (root.currentIndex === index)
                    root._glideTo(root.view, index, instant);
            });
            return;
        }
        var target = root._centerTarget(v, index);
        if (!instant)
            root._resizeRepair = false;
        if (target === null || Math.abs(target - v.contentY) < 1)
            return;
        if (instant || Theme.reduceMotion || root.followMs <= 0) {
            v.contentY = target;
            root._settleGlide();
            return;
        }
        root._followStartY = v.contentY;
        root._followElapsed = 0;
        root._followDuration = root.glideOverrideMs > 0
            ? root.glideOverrideMs : root.followMs;
        centerGlide.start();
    }

    function _followTick() {
        var v = root.view;
        if (!v || root.currentIndex < 0 || root.isLanding) {
            centerGlide.stop();
            return;
        }
        var target = root._centerTarget(v, root.currentIndex);
        if (target === null) {
            centerGlide.stop();
            return;
        }
        root._followElapsed += centerGlide.frameTime * 1000;
        var t = root._followDuration > 0
            ? Math.min(1, root._followElapsed / root._followDuration) : 1;
        var eased = 1 - Math.pow(1 - t, 3);
        v.contentY = root._followStartY + (target - root._followStartY) * eased;
        if (t >= 1 || Math.abs(v.contentY - target) < 0.8) {
            v.contentY = target;
            centerGlide.stop();
            root.glideOverrideMs = 0;
            root._settleGlide();
        }
    }

    /// Stop the glide (a manual scroll owns the viewport until the host
    /// re-follows).
    function stopGlide() {
        centerGlide.stop();
        root.glideOverrideMs = 0;
        root._resizeRepair = false;
    }

    /// Centre the cursor through Qt's own geometry. A resize invalidates the
    /// delegate positions for a few frames, so the caller ticks this until the
    /// layout settles.
    function _centerNow() {
        var v = root.view;
        if (!v || root.currentIndex < 0 || root.isLanding)
            return;
        if (!v.itemAtIndex(root.currentIndex))
            root._landUnrealised(v, root.currentIndex);
        v.positionViewAtIndex(root.currentIndex, ListView.Center);
        if (root.memoryKey)
            root._scheduleRemember();
    }

    /// Public re-centre seam (hosts/layout repairs).
    function recenter(duration) {
        root._centerNow();
    }

    /// The end-of-list settle: `contentHeight` is an estimate until the last
    /// delegates exist. If the cursor ended outside the viewport, let Qt's
    /// `Contain` reveal it, then clamp and remember.
    function _settleGlide() {
        var v = root.view;
        if (!v || root.currentIndex < 0)
            return;
        var item = v.itemAtIndex(root.currentIndex);
        if (!item)
            return;
        if (item.y < v.contentY
                || item.y + item.height > v.contentY + v.height)
            v.positionViewAtIndex(root.currentIndex, ListView.Contain);
        root._clampY(v);
        if (root.memoryKey)
            root._scheduleRemember();
    }

    // ── Landings ─────────────────────────────────────────────────────────

    /// True while a landing runs: the root index handler must not start a
    /// glide the landing is already positioning.
    property bool _instantFollow: false
    readonly property bool isLanding: root._instantFollow
    /// A resize can update the row geometry over several frames; every
    /// contentHeight change re-centres instead of leaving the cursor behind.
    property bool _resizeRepair: false

    /// Instant landing (semantic jumps: `gg`/`G`, search, restores, reset).
    function land(index) {
        root._land(index, true);
    }

    function _land(index, withSettle) {
        if (index < 0)
            return;
        centerGlide.stop();
        root._resizeRepair = false;
        root._instantFollow = true;
        root.currentIndex = index;
        var v = root.view;
        if (v && v.count > 0) {
            v.currentIndex = index;
            if (!v.itemAtIndex(index))
                root._landUnrealised(v, index);
            var target = root._centerTarget(v, index);
            if (target !== null)
                v.contentY = target;
            root._clampY(v);
        }
        var settle = withSettle && v && v.count > 0 && index === v.count - 1;
        Qt.callLater(function () {
            root._instantFollow = false;
            if (settle)
                root._settleGlide();
        });
    }

    /// A surface returning to screen calls this: land on the cursor.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing landing: land exactly and centre the row.
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        root._pendingFollow = i;
        root.land(i);
        Qt.callLater(function () {
            if (root.currentIndex !== i) {
                if (root._pendingFollow === i)
                    root._pendingFollow = -1; // another landing owns it now
                return;
            }
            var v = root.view;
            if (!v || v.count === 0)
                return; // `onCountChanged` finishes the follow
            root._pendingFollow = -1;
            root._glideTo(root.view, i, false);
        });
    }

    /// The `goto` intent that hit an empty view; consumed by `onCountChanged`.
    property int _pendingFollow: -1

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        centerGlide.stop();
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
            return false;
        var rows = Math.max(1, Math.floor(
            root.view.height * (fraction === undefined ? 0.5 : fraction) / rowH));
        return root.moveStep(down ? "move_down" : "move_up", rows);
    }

    function activate() {
        if (root.currentIndex >= 0)
            root.itemActivated(root.currentIndex);
    }

    signal itemActivated(int index)

    // ── Memory (UI-54) ───────────────────────────────────────────────────

    /// Remember/restore cursor + offset under this key ("" = off).
    property string memoryKey: ""
    property int memoryScope: -1

    /// Restore the remembered cursor + offset (false when none).
    function restore() {
        applySavedLayout();
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        root.land(index);
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

    /// §4.3: a filter/order swap rebuilds the rows under the same indices, so
    /// the next payload lands on row 0 with the offset at the top.
    property bool resetPending: false

    /// The pre-payload half of a structural swap: park the cursor and top the
    /// view before the model churns.
    function resetView() {
        root.resetPending = true;
        centerGlide.stop();
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

    /// The landing itself: row 0 centred, and the memory agrees.
    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    // ── Row state (VI-4 / ARC-I-4) ───────────────────────────────────────

    property bool selectable: false
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

    function toggleSelection() {
        rowState.toggle();
    }

    function selectAllRows() {
        rowState.selectAllRows();
    }

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

    function clearSelection() {
        rowState.clear();
    }

    function selected(index) {
        return rowState.selected(index);
    }

    function selectionIds(field) {
        return rowState.ids(field);
    }

    function selectionStarred(field) {
        return rowState.starred(field);
    }

    function markStarred(on) {
        rowState.markStarred(on);
    }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    SearchContract {
        id: searchContract
        model: root.model
        view: root.view
        fields: root.searchFields
    }

    function searchRows() {
        return searchContract.rows();
    }

    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.land(row);
    }

    // ── Rows (UI-58) ─────────────────────────────────────────────────────

    /// The host's nominal row fraction of the sizing height (0 = delegates
    /// own their heights).
    property real rowRatio: 0
    /// Host-supplied sizing height (0 = this body's own viewport).
    property real rowRefHeight: 0
    readonly property real rowViewHeight: root.rowRefHeight > 0
        ? root.rowRefHeight : root.height
    /// Adjustable row-height multiplier for compact hosts.
    property real rowHeightScale: 1.0
    /// The row baseline captured on the first real measure.
    property real _rowBase: 0

    function adaptiveRowFor(viewHeight) {
        return Theme.adaptiveRow(viewHeight, root.rowRatio, root._rowBase)
            * root.rowHeightScale;
    }

    readonly property real adaptiveRowHeight: adaptiveRowFor(root.rowViewHeight)

    Component.onCompleted: {
        captureRowBase();
        applySavedLayout();
    }
    onHeightChanged: {
        if (root.rowRefHeight <= 0)
            captureRowBase();
        root._scheduleResizeRecenter();
    }
    onRowRefHeightChanged: captureRowBase()
    onMemoryKeyChanged: applySavedLayout()

    // A resize changes the row/cell unit while `contentY` stays put; coalesce
    // the ticks, then glide the cursor back to the centre once the row unit
    // settles. Width is host-gated (fixed-row lists keep their elide).
    onWidthChanged: if (root.recenterOnWidth) root._scheduleResizeRecenter()
    onAdaptiveRowHeightChanged: root._scheduleResizeRecenter()
    onCellHeightChanged: root._scheduleResizeRecenter()
    onColumnsChanged: root._scheduleResizeRecenter()

    property bool recenterOnWidth: root.layout === "grid"
    property real rowSpacing: 0
    property real contentHeightHint: 0

    property int _resizeTicks: 0

    function _scheduleResizeRecenter() {
        if (root.currentIndex < 0)
            return;
        root.stopGlide();
        root._resizeRepair = true;
        root._resizeTicks = 10;
        root._centerNow();
        resizeRecenter.restart();
    }

    Timer {
        id: resizeRecenter
        interval: 50
        repeat: true
        onTriggered: {
            root._centerNow();
            if (--root._resizeTicks <= 0) {
                root._resizeRepair = false;
                resizeRecenter.stop();
            }
        }
    }

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && root.rowViewHeight > 0)
            root._rowBase = root.rowViewHeight * root.rowRatio;
    }

    // Auto-fill inside Layout parents; anchor callers size it themselves.
    Layout.fillWidth: true
    Layout.fillHeight: true

    // ── The views ────────────────────────────────────────────────────────

    ScrollMemory {
        id: scroll
    }

    // Half a viewport above row 0 and below the last row: the cursor can be
    // centred even at the ends, and the gap is intentional.
    Component {
        id: edgeSpacer
        Item {
            width: root.width
            height: root._edgeMargin
        }
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
            header: edgeSpacer
            footer: edgeSpacer
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: true
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
            topMargin: root._edgeMargin
            bottomMargin: root._edgeMargin
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: true
        }
    }

    // The view is a follower: its transients are never adopted.
    Connections {
        target: root.view
        function onContentYChanged() {
            if (root.memoryKey && !centerGlide.running)
                root._scheduleRemember();
        }
        function onMovementStarted() {
            root.stopGlide();
        }
        function onContentHeightChanged() {
            if (centerGlide.running || root.isLanding)
                return;
            if (root._resizeRepair && root.currentIndex >= 0)
                root._centerNow();
            else
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
            if (target.count === 0)
                return;
            if (root.currentIndex < 0) {
                if (scroll.savedIndex(root.memoryKey) < 0)
                    root._land(0, false);
            } else if (root.currentIndex > target.count - 1) {
                root._land(target.count - 1, false);
            } else if (target.currentIndex !== root.currentIndex) {
                root._land(root.currentIndex, false);
            }
            if (root.memoryKey)
                root.remember();
            if (root._pendingFollow >= 0
                    && root._pendingFollow === root.currentIndex
                    && target.count > root._pendingFollow) {
                var i = root._pendingFollow;
                root._pendingFollow = -1;
                Qt.callLater(function () {
                    if (root.currentIndex === i && root.view && root.view.count > 0)
                        root._glideTo(root.view, i, false);
                });
            }
        }
    }
}
