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
import "Theme.js" as Theme

// The shared scroll body every cursor list reuses (Albums/Songs/Artists via
// `ScrollPanel`, the search columns, the fullscreen queue, the drawer and the
// album page). It owns the one seam that was copied everywhere:
//
//   - the UI-48 overflow stage: the list bleeds `bleed` into the gutters so a
//     row highlight can go full-bleed while rows still clip vertically;
//   - the cursor (`currentIndex`), the view-owned follow (UI-41) and the
//     scroll memory (UI-54) through a single `ListScroll`.
//
// The follow is the ListView's own highlight machinery (SCROLLCONTENTSTUDY.md):
// an invisible 1x1 highlight is the follow target, `highlightMoveVelocity`
// gives a constant-speed SmoothedAnimation that retargets without re-easing
// and respects flick velocity, and the preferred range keeps the cursor in
// view with a margin. No `contentY` animation exists to fight
// `positionViewAtIndex` jumps (gg/G, recall, select-all) or user input.
//
// Drop it inside any parent Rectangle/Item: with a Layout parent it fills the
// cell (the attached fills below), with an anchor parent the caller anchors
// it (`anchors.fill: parent`); the list itself always fills the component.
Item {
    id: root

    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: 0
    property var model: null
    property Component rowDelegate: null
    /// ARC-UI-2 M1: the model roles `/` searches, in paint order (the
    /// selectable-text contract, SEARCHACTIONSTUDY §3). Hosts override;
    /// cells for absent roles come back empty. Chrome never matches.
    property var searchFields: ["title"]
    /// ARC-UI-2 M2: the live `/` session — the query the row cells paint
    /// their blocks for, and the current candidate row (for `CurSearch`).
    /// Held by whoever owns the session (`LineSearch` pushes it down).
    property string searchQuery: ""
    property int searchCurrentRow: -1

    /// UI-58: the host's nominal row fraction of this viewport (0 = the
    /// delegates own their heights and nothing adapts).
    property real rowRatio: 0
    /// The row baseline captured on the first real measure — the default
    /// window geometry (Main starts at `Screen.height * 0.72`).
    property real _rowBase: 0
    /// UI-58: the row height to paint — scales with the viewport inside the
    /// 30% band, cuts whole rows past it (see `Theme.adaptiveRow`).
    readonly property real adaptiveRowHeight: Theme.adaptiveRow(list.height, root.rowRatio, root._rowBase)

    Component.onCompleted: captureRowBase()
    onHeightChanged: captureRowBase()

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && list.height > 0)
            root._rowBase = list.height * root.rowRatio;
    }
    /// UI-41: the view-owned follow speed (px/s) and the preferred band the
    /// cursor is kept in (fractions of the viewport). `reduceMotion` snaps.
    property real followVelocity: 1400
    property real followBegin: 0.15
    property real followEnd: 0.85
    /// VI-4: `v`/`V` row selection. The model owns the state; hosts gate it
    /// (e.g. only when the Alt verb kind exists) and keep their own cursor.
    property bool selectable: false
    /// UI-54: remember/restore cursor + offset under this key ("" = off).
    property string memoryKey: ""
    property int memoryScope: -1

    // Auto-fill inside Layout parents; anchor callers size it themselves.
    Layout.fillWidth: true
    Layout.fillHeight: true

    property alias currentIndex: list.currentIndex
    property alias count: list.count
    /// The ListView itself (itemAtIndex, positionViewAtIndex, …).
    property alias view: list

    signal itemActivated(int index)

    ListScroll {
        id: scroll
    }

    // VI-4: anchored visual selection over the cursor range.
    Selection {
        id: sel
    }

    readonly property bool hasSelection: sel.hasSelection
    readonly property int selectionCount: sel.count(list.currentIndex)

    /// `v`: start a selection on the cursor row, or end the current one.
    function toggleSelection() {
        if (!root.selectable || list.count === 0 || list.currentIndex < 0)
            return;
        if (sel.hasSelection)
            sel.clear();
        else
            sel.start(list.currentIndex);
    }

    /// `V`: the whole list. The cursor is the host's (a binding in the search
    /// columns), so the host moves it and this only widens the anchor.
    function selectAllRows() {
        if (!root.selectable || list.count === 0)
            return;
        sel.all();
    }

    function clearSelection() {
        sel.clear();
    }

    function selected(index) {
        return sel.selected(list.currentIndex, index);
    }

    /// Ids of the selected rows ("" when there is no selection).
    function selectionIds(field) {
        return sel.ids(root.model, list.currentIndex, field);
    }

    /// Instant `Contain` for semantic jumps (toolbar reset, select-all,
    /// reorder centring). Cursor moves rely on the view's follow; with no
    /// custom animation left there is nothing this teleport can fight.
    function ensureVisible(index) {
        var i = index === undefined ? list.currentIndex : index;
        if (i >= 0)
            list.positionViewAtIndex(i, ListView.Contain);
    }

    function activate() {
        if (list.currentIndex >= 0)
            root.itemActivated(list.currentIndex);
    }

    /// ARC-UI-2 M1: the cells of the rows the user can actually *see* —
    /// matching is viewport-scoped, so a match scrolled out of the viewport
    /// comes back as an empty placeholder (never matches) and `/` can never
    /// afford a jump the eye cannot verify without scrolling. The prompt
    /// snapshots this band once, when it opens.
    function searchRows() {
        var out = [];
        if (!root.model || !root.model.get)
            return out;
        var first = list.indexAt(list.contentX + list.width * 0.5, list.contentY);
        var last = list.indexAt(list.contentX + list.width * 0.5,
            list.contentY + Math.max(0, list.height - 1));
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
                var v = item[root.searchFields[f]];
                cells.push(v === undefined || v === null ? "" : String(v));
            }
            out.push(cells);
        }
        return out;
    }

    /// ARC-UI-2 M1: land the view cursor on a matching row; the view-owned
    /// follow keeps it visible (the same path `gg`/`G` take).
    function searchJump(row) {
        if (row < 0 || row >= list.count)
            return;
        list.currentIndex = row;
        ensureVisible(row);
    }

    /// UI-54: restore the remembered cursor + offset (false when empty).
    function restore() {
        return scroll.recall(root.memoryKey, list);
    }

    function remember(scope) {
        if (root.memoryKey)
            scroll.remember(root.memoryKey, list, scope === undefined ? root.memoryScope : scope);
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
            // With bleed the stage clips for it; otherwise the list does.
            clip: root.bleed <= 0
            model: root.model
            delegate: root.rowDelegate
            boundsBehavior: Flickable.StopAtBounds

            // The invisible follow target (the visible row highlight is the
            // delegate's). Duration stays -1 so the velocity drives the move;
            // `reduceMotion` collapses it to a snap.
            highlight: Component {
                Rectangle {
                    width: 1
                    height: 1
                    color: "transparent"
                }
            }
            highlightFollowsCurrentItem: true
            highlightMoveVelocity: root.followVelocity
            highlightMoveDuration: Theme.reduceMotion ? 0 : -1
            preferredHighlightBegin: height * root.followBegin
            preferredHighlightEnd: height * root.followEnd
            highlightRangeMode: ListView.ApplyRange
        }
    }

    Connections {
        target: list
        function onContentYChanged() {
            if (root.memoryKey)
                scroll.remember(root.memoryKey, list, root.memoryScope);
        }
        function onCurrentIndexChanged() {
            if (root.memoryKey)
                scroll.remember(root.memoryKey, list, root.memoryScope);
        }
    }
}
