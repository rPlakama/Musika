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

// The shared scroll body — SCROLLMODELANALYSIS.md is the spec.
//
// Ownership: the host owns the cursor *semantics* through `currentIndex`;
// this body owns the viewer (the active ListView/GridView, the follow, the
// memory hooks, the VI-4 selection, the `/` search fields and the adaptive
// row height); `ScrollMemory` is the only persistent state.
//
// Follow: the viewport *is* animated to the cursor's centre — the glide
// (`followMs`, `Theme.motion.scroll`) with a small OutBack spring, clamped
// at the content ends (Neovim `'scrolloff': 0.5` semantics, no padding).
// This is the only follow: the highlight-band/velocity era is gone —
// `ApplyRange` enforced the band *instantly*, so a one-row move snapped
// even with a 300 ms move duration (probed; CRITIQUESCROLL SC-1).
//
// Landings: `goto(row)` (centre; the host-facing one) and `land(row)`
// (instant `Contain`, the semantic jumps: `gg`/`G`, search, restores,
// reset). **Hosts never touch `view.contentY`/`positionViewAtIndex`** —
// the body is the only writer, and `_instantFollow` shields a landing
// from the root index handler's glide.
//
// Guards: the view is a follower, never a source — its own refill/clamp
// transients are overwritten with the body's cursor; a filter/order swap
// raises `resetView()` and the payload consumes it as a top landing; an
// empty view defers the carried cursor until `onCountChanged`.
//
// ARC-UI-3: `layout` picks the active view; both share the stage, memory,
// selection, search contract and adaptive sizing. Drop it inside any
// parent Rectangle/Item (Layout or anchors).
Item {
    id: root

    // ── Model & roles ────────────────────────────────────────────────────

    property var model: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: 0
    property Component rowDelegate: null
    /// ARC-UI-3: the grid cell delegate (cover + caption, like ArtistEntry).
    property Component cellDelegate: null

    /// ARC-UI-2 M1: the model roles `/` searches, in paint order (the
    /// selectable-text contract, SEARCHACTIONSTUDY §3). Hosts override;
    /// cells for absent roles come back empty. Chrome never matches.
    property var searchFields: ["title"]
    /// ARC-UI-2 M2: the live `/` session — the query the row cells paint
    /// their blocks for, and the current candidate row (for `CurSearch`).
    /// Held by whoever owns the session (`LineSearch` pushes it down).
    property string searchQuery: ""
    property int searchCurrentRow: -1

    // ── Layout (list ⇄ grid) ─────────────────────────────────────────────

    /// ARC-UI-3: supported layouts in toggle order ("list", "grid"); a single
    /// entry means the surface never toggles.
    property var layouts: ["list"]
    /// The active body. The host sets its default; the saved layout (below)
    /// can override it on open.
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

    // ── Cursor & follow ──────────────────────────────────────────────────

    /// The cursor: the single source of truth (§1). Hosts write it; the
    /// live view mirrors it (an alias cannot follow the active view).
    property int currentIndex: -1
    onCurrentIndexChanged: {
        // A landing (`land`) owns the view: it positions and mirrors the
        // index itself; starting a glide here would fight it and leave a
        // corrupted contentY behind.
        if (root._instantFollow)
            return;
        var v = root.view;
        if (v && v.count > 0 && v.currentIndex !== root.currentIndex) {
            v.currentIndex = root.currentIndex;
            root._glideTo(v, root.currentIndex);
        }
    }

    // LYR-8: the centred viewport glide — the one follow. It settles with
    // a small OutBack overshoot (a spring at the stop) capped tight so a
    // one-row step barely breathes past the line and retargets mid-flight
    // stay stable. The content-end clamp eats the overshoot there (the
    // scrolloff edge rule), so the spring is a mid-list effect; the
    // `onFinished` settle covers `contentHeight` estimate growth.
    NumberAnimation {
        id: centerGlide
        target: root.view
        property: "contentY"
        duration: root.followMs
        easing.type: Easing.OutBack
        easing.overshoot: 1.7
        onFinished: root._settleGlide()
    }

    /// Centre the cursor over `followMs` — the delegate itself is the target
    /// (lyric rows have variable heights); an unrealised line falls back to
    /// an instant `Center`.
    function _glideTo(v, index) {
        if (!v || v.count === 0 || index < 0 || root._instantFollow)
            return;
        var item = v.itemAtIndex(index);
        if (!item) {
            // Unrealised row (a far landing): iterative, height-aware reveal
            // (CRITIQUESCROLL SC-4/P3) — Qt's `Contain` on a scrolled list
            // parks async offsets (`-4`, `-96`, `-920`; probed) and assumes
            // fixed heights, so we step toward the row using the observed
            // local row unit instead, then glide to the centre once the row
            // exists.
            centerGlide.stop();
            root._landUnrealised(v, index);
            Qt.callLater(function () {
                if (root.currentIndex === index)
                    root._glideTo(root.view, index);
            });
            return;
        }
        var limit = Math.max(0, v.contentHeight - v.height);
        var target = Math.max(0, Math.min(limit,
            item.y + item.height * 0.5 - v.height * 0.5));
        if (Math.abs(target - v.contentY) < 1)
            return;
        if (Theme.reduceMotion) {
            v.contentY = target;
            return;
        }
        centerGlide.stop();
        centerGlide.from = v.contentY;
        centerGlide.to = target;
        centerGlide.start();
    }

    /// Stop the glide (a manual scroll owns the viewport until the host
    /// re-follows).
    function stopGlide() {
        centerGlide.stop();
    }

    /// The end-of-list settle: `ListView.contentHeight` is an estimate until
    /// the bottom delegates exist, so a glide toward the last rows can stop
    /// short with the cursor under the viewport's end. Qt's own `Contain`
    /// uses the true content; it only moves the shortfall.
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
    }

    /// The active view (itemAtIndex, positionViewAtIndex, …).
    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: Qt.callLater(root.landCursor)
    onLayoutChanged: {
        // Only the layout field — a restore flips the layout before the
        // cursor is back, and a full save here would clobber the memory.
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    /// The follow duration (ms): the rows drift up/down softly as the
    /// cursor steps, and the pane fade binds to the same value. 0 snaps
    /// (`reduceMotion` also snaps, in `_glideTo`).
    property int followMs: Theme.motion.scroll
    /// Extra spacing between list rows (0 = the delegate owns it); grid
    /// cells are placed by `cellHeight`.
    property real rowSpacing: 0

    /// True while a landing runs: the root index handler must not start a
    /// glide the landing is already positioning (`land` owns the view).
    property bool _instantFollow: false

    /// Keep `contentY` inside the content bounds (Qt's async `Contain`
    /// layouts can park it outside; probed).
    function _clampY(v) {
        var limit = Math.max(0, v.contentHeight - v.height);
        if (v.contentY < 0 || v.contentY > limit)
            v.contentY = Math.max(0, Math.min(limit, v.contentY));
    }

    /// Bring an unrealised row into view without Qt's `Contain`/`Center`
    /// (both park async offsets on a scrolled list — `-4`, `-96`, `-920`
    /// probed): `Beginning` anchors the row at the viewport top, we clamp
    /// the result into the content bounds, and the caller centres it. No
    /// index→pixel estimation, so variable row heights are exact
    /// (CRITIQUESCROLL SC-4/P3).
    function _landUnrealised(v, index) {
        v.positionViewAtIndex(index, ListView.Beginning);
        root._clampY(v);
    }

    /// The instant landing (semantic jumps: `gg`/`G`, search, restores, the
    /// filter reset). Positions and mirrors the cursor itself; the running
    /// glide is killed first, and the `Contain` result is repaired when Qt
    /// leaves a negative contentY on a scrolled list (probed). An empty
    /// view carries the cursor and lands it on the next count change.
    function land(index) {
        if (index < 0)
            return;
        centerGlide.stop();
        root._instantFollow = true;
        root.currentIndex = index;
        var v = root.view;
        if (v && v.count > 0) {
            v.currentIndex = index;
            if (v.itemAtIndex(index))
                v.positionViewAtIndex(index, ListView.Contain);
            else
                root._landUnrealised(v, index);
            root._clampY(v);
        }
        Qt.callLater(function () { root._instantFollow = false; });
    }

    /// A surface returning to screen (segment switch, page pop) calls this:
    /// if the cursor is far from the viewport — or the view lost its place
    /// while hidden — it lands instantly instead of walking back.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing landing: `goto(row)` lands exactly (`Contain`) and
    /// then glides the row to the pane centre on the next tick, once the
    /// layout is true. Use `land(row)` for semantic jumps that must stay
    /// instant. **Hosts never touch `view.contentY`/`positionViewAtIndex`**
    /// — the running glide would override the teleport (CRITIQUESCROLL
    /// SC-2: the queue's current track ended up below the viewport that
    /// way).
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        root.land(i);
        Qt.callLater(function () {
            if (root.currentIndex === i && root.view && root.view.count > 0)
                root._glideTo(root.view, i);
        });
    }

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        centerGlide.stop();
        if (root.view)
            root.view.positionViewAtBeginning();
    }

    /// `gg`/`G`: instant edge landing — the follow band has nothing to
    /// animate once the target is already on screen.
    function edgeJump(last) {
        if (root.count === 0)
            return;
        root.land(last ? root.count - 1 : 0);
    }

    /// Shared cursor step: in grid mode the wall walks by `columns` (no
    /// wrap-around — GridView's own moveCurrentIndex* crosses rows), in list
    /// mode it is a plain ±1. Returns false when the edge blocks the move,
    /// so Home can hand `h` to the sidebar from the leftmost column.
    function moveStep(action) {
        if (root.count === 0)
            return false;
        var cols = root.layout === "grid" ? Math.max(1, root.columns) : 1;
        var column = root.currentIndex % cols;
        var target = root.currentIndex;
        if (action === "move_left") {
            if (column === 0)
                return false;
            target -= 1;
        } else if (action === "move_right") {
            if (column === cols - 1 || root.currentIndex + 1 >= root.count)
                return false;
            target += 1;
        } else if (action === "move_up") {
            if (root.currentIndex - cols < 0)
                return false;
            target -= cols;
        } else if (action === "move_down") {
            if (root.currentIndex + cols >= root.count)
                return false;
            target += cols;
        } else {
            return false;
        }
        root.currentIndex = target;
        return true;
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

    /// Restore the remembered cursor + offset (false when none). The saved
    /// layout is applied first so the right view holds the offset; the
    /// cursor lands instantly (a long return must not walk) and `recall`
    /// then writes the exact offset. An empty view lands the cursor on the
    /// next count change.
    function restore() {
        applySavedLayout();
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        root.land(index);
        scroll.recall(root.memoryKey, root.view);
        return true;
    }

    function remember(scope) {
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view,
                scope === undefined ? root.memoryScope : scope, root.layout,
                root.currentIndex);
    }

    /// §4.3: a filter/order swap rebuilds the rows under the same indices,
    /// so the next payload must land on row 0 with the offset at the top.
    /// Hosts raise this before reloading; the payload consumes it.
    property bool resetPending: false

    function resetView() {
        root.resetPending = true;
    }

    /// The landing itself: row 0, top, and the memory agrees (so the next
    /// restore cannot recall the old spot). Cursor and position land in the
    /// same tick, or the follow would walk the whole list back (§4.2).
    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
        root.view.positionViewAtBeginning();
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    // ── Row state (VI-4 / ARC-I-4) ───────────────────────────────────────

    // The model owns the state; hosts gate it (e.g. only when the Alt verb
    // kind exists) and keep their own cursor. The logic lives in `RowState`
    // (CRITIQUESCROLL SC-7), fed by the body's cursor.
    property bool selectable: false

    RowState {
        id: rowState
        model: root.model
        cursor: root.currentIndex
        selectable: root.selectable
    }

    readonly property bool hasSelection: rowState.hasSelection
    readonly property int selectionCount: rowState.selectionCount

    /// `v`: start a selection on the cursor row, or end the current one.
    function toggleSelection() {
        rowState.toggle();
    }

    /// `V`: the whole list — the host moves the cursor first, this only
    /// widens the anchor.
    function selectAllRows() {
        rowState.selectAllRows();
    }

    function clearSelection() {
        rowState.clear();
    }

    function selected(index) {
        return rowState.selected(index);
    }

    /// Ids of the selected rows ([] when there is no selection).
    function selectionIds(field) {
        return rowState.ids(field);
    }

    /// The selected rows' `starred` flags, aligned with `selectionIds`.
    function selectionStarred(field) {
        return rowState.starred(field);
    }

    /// ARC-I-4: the in-place favourite write (see `RowState`).
    function markStarred(on) {
        rowState.markStarred(on);
    }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    // The viewport-scoped row snapshot lives in `SearchContract`
    // (CRITIQUESCROLL SC-7); the session state and the jump stay here.
    SearchContract {
        id: searchContract
        model: root.model
        view: root.view
        fields: root.searchFields
    }

    /// The rendered rows the `/` prompt snapshots (see `SearchContract`).
    function searchRows() {
        return searchContract.rows();
    }

    /// Land the view cursor on a matching row; the instant landing keeps
    /// the jump from walking (the same path `gg`/`G` take).
    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.land(row);
    }

    // ── Rows (UI-58) ─────────────────────────────────────────────────────

    /// The host's nominal row fraction of this viewport (0 = the delegates
    /// own their heights and nothing adapts).
    property real rowRatio: 0
    /// The row baseline captured on the first real measure — the default
    /// window geometry (Main starts at `Screen.height * 0.72`).
    property real _rowBase: 0
    /// The row height to paint — scales with the viewport inside the 18%
    /// band (0.82, 40% sooner than the original), cuts whole rows past it
    /// (see `Theme.adaptiveRow`). Delegates must size from this, never from
    /// raw viewport fractions (§5).
    readonly property real adaptiveRowHeight: Theme.adaptiveRow(height, root.rowRatio, root._rowBase)

    Component.onCompleted: {
        captureRowBase();
        applySavedLayout();
    }
    onHeightChanged: captureRowBase()
    onMemoryKeyChanged: applySavedLayout()

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && root.height > 0)
            root._rowBase = root.height * root.rowRatio;
    }

    // Auto-fill inside Layout parents; anchor callers size it themselves.
    Layout.fillWidth: true
    Layout.fillHeight: true

    // ── The views ────────────────────────────────────────────────────────

    ListScroll {
        id: scroll
    }

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
            // With bleed the stage clips for it; otherwise the view does.
            clip: root.bleed <= 0
            visible: root.layout !== "grid"
            model: root.layout === "grid" ? null : root.model
            delegate: root.rowDelegate
            spacing: root.rowSpacing
            boundsBehavior: Flickable.StopAtBounds
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
        }
    }

    // The view side of the cursor contract (§4.1/§4.2): the active view is
    // a follower; its refill/clamp transients are never adopted.
    Connections {
        target: root.view
        function onContentYChanged() {
            if (root.memoryKey)
                root.remember();
        }
        // A drag owns the viewport: kill any centre glide in flight.
        function onMovementStarted() {
            root.stopGlide();
        }
        function onCurrentIndexChanged() {
            // A landing owns the view while it runs.
            if (target.count === 0 || root._instantFollow)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            if (root.memoryKey)
                root.remember();
        }
        // A payload (re)load: a fresh list lands on row 0 (no memory, the
        // cursor was never set), an index past the new end clamps, and a
        // view that was empty a beat ago lands the carried cursor — all
        // instantly, a reload must not walk the viewport. The row-0 case is
        // the cursor-as-property replacement for the old alias accident
        // (the view defaulted to 0 and the alias silently adopted it).
        function onCountChanged() {
            if (target.count === 0)
                return;
            if (root.currentIndex < 0) {
                // A fresh list lands on row 0 — unless a memory exists, in
                // which case the host's `restore()` owns the first landing
                // (and the guarded remember below never erases it with -1).
                if (scroll.savedIndex(root.memoryKey) < 0)
                    root.land(0);
            } else if (root.currentIndex > target.count - 1) {
                root.land(target.count - 1);
            } else if (target.currentIndex !== root.currentIndex) {
                root.land(root.currentIndex);
            }
            if (root.memoryKey)
                root.remember();
        }
    }
}
