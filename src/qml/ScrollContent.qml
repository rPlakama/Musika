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
// Ownership (§1): the host owns the cursor *semantics* through
// `currentIndex`; this body owns the viewer (the active ListView/GridView,
// the view-owned follow, the memory hooks, the VI-4 selection, the `/`
// search fields and the adaptive row height); `ScrollMemory` is the only
// persistent state, shared per `memoryKey`.
//
// Follow (§3): the view scrolls itself through an invisible 1×1
// `highlight` (`highlightMoveVelocity` on the list; GridView has no
// velocity property, so it rides `highlightMoveDuration`) inside the
// 15%…85% preferred band. Semantic jumps (`gg`/`G`, search, restores, the
// filter reset) land instantly through `ensureVisible`/`edgeJump` so the
// highlight never walks; a landing farther than `followSnapRows` outside
// the visible band snaps even when the host moved the index directly.
//
// Guards (§4): the view is a follower, never a source — its own refill and
// clamp transients (a fresh GridView resets its index to 0 when the model
// lands) are overwritten with the body's cursor; a filter/order swap
// raises `resetView()` and the payload consumes it as a top landing; an
// empty view defers the carried cursor until `onCountChanged`.
//
// ARC-UI-3 (SCROLLGRIDSTUDY.md): `layout` picks the active view; both share
// the stage, band, memory, selection, search contract and adaptive sizing.
// Drop it inside any parent Rectangle/Item: with a Layout parent it fills
// the cell; with an anchor parent the caller anchors it.
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
        var v = root.view;
        if (v && v.count > 0 && v.currentIndex !== root.currentIndex)
            v.currentIndex = root.currentIndex;
    }

    /// The active view (itemAtIndex, positionViewAtIndex, …).
    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: Qt.callLater(root._applyCursor)
    onLayoutChanged: {
        // Only the layout field — a restore flips the layout before the
        // cursor is back, and a full save here would clobber the memory.
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    /// UI-41: the view-owned follow speed (px/s) and the preferred band the
    /// cursor is kept in (fractions of the viewport). `reduceMotion` snaps.
    property real followVelocity: 1400
    property real followBegin: 0.15
    property real followEnd: 0.85
    /// A landing farther than this many rows outside the visible band
    /// catches instantly — the highlight walk over dozens of rows reads as
    /// lag (and blanks the viewport while it travels).
    property int followSnapRows: 15

    /// Mirror the body's cursor onto the active view (`position` also
    /// teleports the viewport — swaps and recalls, not payload reloads).
    function _applyCursor(position) {
        var v = root.view;
        if (!v || v.count === 0 || root.currentIndex < 0)
            return;
        if (v.currentIndex !== root.currentIndex)
            v.currentIndex = root.currentIndex;
        if (position)
            v.positionViewAtIndex(root.currentIndex, ListView.Contain);
    }

    /// §3: a far landing is a jump, not a step. Called on every view-side
    /// index change, after the cursor mirror above.
    function _snapIfFar(v) {
        if (!v || root.currentIndex < 0 || root.followSnapRows <= 0)
            return;
        var middle = v.contentX + v.width * 0.5;
        var top = v.indexAt(middle, v.contentY);
        var bottom = v.indexAt(middle, v.contentY + Math.max(0, v.height - 1));
        if (top < 0)
            top = 0;
        if (bottom < 0)
            bottom = v.count - 1;
        if (root.currentIndex < top - root.followSnapRows
                || root.currentIndex > bottom + root.followSnapRows)
            v.positionViewAtIndex(root.currentIndex, ListView.Contain);
    }

    /// Instant `Contain` for semantic jumps (gg/G, search, select-all,
    /// restores). Cursor moves rely on the view's follow; there is no
    /// custom animation left for a same-tick teleport to fight.
    function ensureVisible(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i >= 0 && root.view)
            root.view.positionViewAtIndex(i, ListView.Contain);
    }

    /// `gg`/`G`: instant edge jump — the follow band has nothing to animate
    /// once the target is already on screen.
    function edgeJump(last) {
        if (root.count === 0)
            return;
        root.currentIndex = last ? root.count - 1 : 0;
        ensureVisible(root.currentIndex);
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
    /// layout is applied first so the right view holds the offset; an empty
    /// view lands the cursor on the next count change.
    function restore() {
        applySavedLayout();
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        root.currentIndex = index;
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
        root.currentIndex = 0;
        root.view.positionViewAtBeginning();
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    // ── Selection (VI-4) ─────────────────────────────────────────────────

    // The model owns the state; hosts gate it (e.g. only when the Alt verb
    // kind exists) and keep their own cursor.
    property bool selectable: false

    Selection {
        id: sel
    }

    readonly property bool hasSelection: sel.hasSelection
    readonly property int selectionCount: sel.count(root.currentIndex)

    /// `v`: start a selection on the cursor row, or end the current one.
    function toggleSelection() {
        if (!root.selectable || root.count === 0 || root.currentIndex < 0)
            return;
        if (sel.hasSelection)
            sel.clear();
        else
            sel.start(root.currentIndex);
    }

    /// `V`: the whole list — the host moves the cursor first, this only
    /// widens the anchor.
    function selectAllRows() {
        if (!root.selectable || root.count === 0)
            return;
        sel.all();
    }

    function clearSelection() {
        sel.clear();
    }

    function selected(index) {
        return sel.selected(root.currentIndex, index);
    }

    /// Ids of the selected rows ("" when there is no selection).
    function selectionIds(field) {
        return sel.ids(root.model, root.currentIndex, field);
    }

    /// The selected rows' `starred` flags, aligned with `selectionIds`.
    function selectionStarred(field) {
        return sel.starred(root.model, root.currentIndex, field);
    }

    /// ARC-I-4: write the favourite state back into the model in place —
    /// the cursor row, or the whole selected range while a selection is
    /// live — so the affected rows' hearts track the Alt verb with no
    /// reload.
    function markStarred(on) {
        if (!root.model || !root.model.setProperty || root.currentIndex < 0)
            return;
        var first = root.hasSelection
            ? Math.min(sel.anchor, root.currentIndex) : root.currentIndex;
        var last = root.hasSelection
            ? Math.max(sel.anchor, root.currentIndex) : root.currentIndex;
        for (var i = first; i <= last; ++i)
            root.model.setProperty(i, "starred", on === true);
    }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    /// The cells of the rows the user can actually *see* — matching is
    /// viewport-scoped, so a match scrolled out of the viewport comes back
    /// as an empty placeholder (never matches) and `/` can never afford a
    /// jump the eye cannot verify. The prompt snapshots this band once,
    /// when it opens. Probing both corners keeps the whole last row of a
    /// grid inside the band (a single mid-x line under-reports its
    /// right-hand cells).
    function searchRows() {
        var out = [];
        if (!root.model || !root.model.get || !root.view)
            return out;
        var v = root.view;
        var first = v.indexAt(v.contentX + Math.max(1, v.width * 0.5), v.contentY);
        var last = v.indexAt(v.contentX + Math.max(1, v.width - 1),
            v.contentY + Math.max(0, v.height - 1));
        if (first < 0)
            first = 0;
        if (last < 0)
            last = root.model.count - 1;
        for (var i = 0; i < root.model.count; ++i) {
            if (i < first || i > last) {
                out.push([""]);
                continue;
            }
            var item = root.model.get(i);
            var cells = [];
            for (var f = 0; f < root.searchFields.length; ++f) {
                var value = item[root.searchFields[f]];
                cells.push(value === undefined || value === null ? "" : String(value));
            }
            out.push(cells);
        }
        return out;
    }

    /// Land the view cursor on a matching row; the instant `Contain` keeps
    /// the jump from walking (the same path `gg`/`G` take).
    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.currentIndex = row;
        ensureVisible(row);
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

    // One invisible follow target shared by both views (the visible row
    // highlight is the delegate's). Duration stays -1 in list mode so the
    // velocity drives the move; the grid has no velocity property, so it
    // rides a duration token instead. `reduceMotion` collapses both.
    Component {
        id: followHighlight
        Rectangle {
            width: 1
            height: 1
            color: "transparent"
        }
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
            boundsBehavior: Flickable.StopAtBounds

            highlight: followHighlight
            highlightFollowsCurrentItem: true
            highlightMoveVelocity: root.followVelocity
            highlightMoveDuration: Theme.reduceMotion ? 0 : -1
            preferredHighlightBegin: height * root.followBegin
            preferredHighlightEnd: height * root.followEnd
            highlightRangeMode: ListView.ApplyRange
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

            highlight: followHighlight
            highlightFollowsCurrentItem: true
            // SCROLLGRIDSTUDY §3: GridView has no `highlightMoveVelocity`;
            // the duration token is the nearest documented follow.
            highlightMoveDuration: Theme.reduceMotion ? 0 : Theme.motion.standard
            preferredHighlightBegin: height * root.followBegin
            preferredHighlightEnd: height * root.followEnd
            highlightRangeMode: GridView.ApplyRange
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
        function onCurrentIndexChanged() {
            if (target.count === 0)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            root._snapIfFar(target);
            if (root.memoryKey)
                root.remember();
        }
        // A payload (re)load: an index past the new end clamps, and a view
        // that was empty a beat ago lands the carried cursor.
        function onCountChanged() {
            if (target.count === 0)
                return;
            if (root.currentIndex > target.count - 1)
                root.currentIndex = target.count - 1;
            root._applyCursor(false);
            if (root.memoryKey)
                root.remember();
        }
    }
}
