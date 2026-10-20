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
import musika
import "Theme.js" as Theme

// Shared scroll panel — the standard shape of every scrollable segment:
//
//   Main   — the surface itself (callers size it; `background` is the fill).
//   A 20%  — the title band, anchored top, horizontal centre: `title` plus any
//            writings declared as children (order cues, counts, chips).
//   B rest — the scroll body, anchored bottom, horizontal centre: the ListView
//            with the eased follow (UI-41), scroll memory (UI-54),
//            loading/empty states and the keyboard cursor.
//
// Callers keep their model, row delegate and payload mapping; the panel owns
// the list and is the single owner of its `contentY`, so no external
// animation can fight the user's scroll.
//
// Panel role (CRITIQUESCROLL SC-6): this is the *enforced* host surface —
// hosts address scrolling through `goto` (and the selection/search/adaptive
// forwards) and never touch `view.contentY`/`positionViewAtIndex`
// themselves; the forwarding members are the contract, not duplication.
Rectangle {
    radius: Theme.radius
    id: root

    // --- Main rectangle ------------------------------------------------------
    property alias background: root.color

    // --- Split geometry ------------------------------------------------------
    // A = this fraction of the height, B = the rest. Both bands are
    // horizontal-centred; B is bottom-anchored to the surface.
    // UI-57: the band sits 30% below the original 0.2 (0.14) and
    // `widthScale` ramps it further as the window narrows.
    property real headerRatio: 0.14
    // UI-48: horizontal gutter the body overflows into (highlight bleed).
    property real bleed: 0

    // UI-57: the header ramps with the panel width, not only its height. The
    // title shrinks toward the headline floor (18 px, never body-text weight)
    // as the window narrows and the band follows it, handing the reclaimed
    // height back to the list. `widthScale` is 1 at `compact` and above,
    // tapering to 0.7 at the `narrow` breakpoint ArtistEntry uses; it is
    // continuous, so resizing — or crossing the breakpoint — never jumps.
    readonly property real widthScale: Theme.widthScale(root.width)
    readonly property real titlePixelSize: {
        // The linear-toward-a-clamp ramp ArtistEntry's description uses.
        var full = Theme.fontSize("headline", root.height);
        return Theme.clamp(full * root.widthScale, 18, full);
    }
    readonly property real effectiveHeaderRatio: root.headerRatio * root.widthScale

    // --- Header (A) ----------------------------------------------------------
    // Children declared on the panel land in the title band.
    default property alias headerContent: header.data
    property string title: ""

    // --- Body (B) ------------------------------------------------------------
    property Component rowDelegate
    property var model: null
    property bool loading: false
    property string emptyText: ""

    // --- Cursor / memory -----------------------------------------------------
    property alias currentIndex: content.currentIndex
    readonly property alias count: content.count
    /// The rendered band: hosts cap marks and mark reads to these rows.
    readonly property alias firstVisibleIndex: content.firstVisibleIndex
    readonly property alias lastVisibleIndex: content.lastVisibleIndex
    readonly property alias visibleRowCount: content.visibleRowCount
    /// Escape hatch for surfaces that need the view itself (lyrics follow).
    /// ARC-UI-3: the body can swap list ⇄ grid, so the alias is read-only.
    readonly property alias view: content.view
    /// VI-4: the selection lives in `ScrollContent`; the panel only forwards
    /// it (the wrappers call these inherited members directly).
    property alias selectable: content.selectable
    readonly property alias hasSelection: content.hasSelection
    readonly property alias selectionCount: content.selectionCount
    property string memoryKey: ""
    property int memoryScope: -1
    property bool _restored: false
    /// Optional handler for extra keyboard actions; returning nothing is fine.
    property var actionHandler: null

    signal itemActivated(int index)
    /// ARC-UI-7.9: forwarded from the shared body for host badges/status.
    signal selectionChanged

    color: Palette.surface

    /// UI-54: first payload after a reopen restores cursor + offset; later
    /// reloads keep the live state. Returns true when a memory was applied.
    function restore() {
        if (root._restored)
            return false;
        root._restored = true;
        return content.restore();
    }

    // VI-4: selection forwards (see `ScrollContent`).
    function toggleSelection() {
        content.toggleSelection();
    }

    /// `V`: the cursor moves here (the panel owns it), the anchor widens.
    function selectAll() {
        if (!selectable || count === 0)
            return;
        currentIndex = count - 1;
        content.selectAllRows();
        goto();
    }

    function clearSelection() {
        content.clearSelection();
    }

    function isSelected(index) {
        return content.selected(index);
    }

    function selectedIds(field) {
        return content.selectionIds(field);
    }

    function selectedStarred(field) {
        return content.selectionStarred(field);
    }

    /// ARC-I-4: the in-place favourite write (see `ScrollContent`).
    function markStarred(on) {
        content.markStarred(on);
    }

    /// UI-10: reset the view on a filter/order swap (see `ScrollContent`).
    property alias resetPending: content.resetPending

    function resetView() {
        content.resetView();
    }

    function resetViewNow() {
        content.resetViewNow();
    }

    function goto(index) {
        content.goto(index);
    }

    function activate() {
        content.activate();
    }

    /// ARC-UI-7.9: Shift+Click anchor extension for the shared row delegates.
    function extendSelectionTo(index) {
        return content.extendSelectionTo(index);
    }

    /// The header-chip Tab stop: a host with a reachable header control sets
    /// `hasHeaderChip`; Tab focuses it, Shift+Tab cycles back (or Tab again),
    /// Enter fires `headerChipActivated`, any other grammar drops the scope
    /// and runs on the list.
    property bool hasHeaderChip: false
    property int headerScope: -1
    signal headerChipActivated()

    /// ARC-UI-2 M1: the search contract forwards to the shared content.
    property alias searchFields: content.searchFields
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow
    /// UI-58: adaptive rows forward too (see `ScrollContent`).
    property alias rowRatio: content.rowRatio
    property alias rowRefHeight: content.rowRefHeight
    property alias rowHeightScale: content.rowHeightScale
    readonly property alias adaptiveRowHeight: content.adaptiveRowHeight
    /// ARC-UI-3: layout modes forward to the shared content (a single-entry
    /// `layouts` keeps the surface list-only).
    property alias layouts: content.layouts
    property alias layout: content.layout
    property alias columns: content.columns
    property alias cellHeight: content.cellHeight
    property alias cellDelegate: content.cellDelegate
    readonly property alias cellWidth: content.cellWidth

    function searchRows() {
        return content.searchRows();
    }

    function searchJump(row) {
        content.searchJump(row);
    }

    function toggleLayout() {
        content.toggleLayout();
    }

    /// ARC-UI-3: the shared 2-D cursor step (grid walks the wall by columns,
    /// list steps ±1); Home probes this for `h`, so the leftmost column can
    /// hand the cursor to the sidebar.
    function moveWithinGrid(action) {
        return content.moveStep(action);
    }

    // Shared cursor grammar: j/k, gg/G, Enter; unknown actions go to
    // `actionHandler` (search columns, queue reorder, …). Cursor moves are
    // followed by the view itself (ScrollContent highlight machinery); gg/G
    // snap instantly via `edgeJump`.
    function handleAction(action) {
        // The header-chip Tab stop: Tab focuses it, Shift+Tab (or Tab again)
        // returns to the list.
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            if (root.hasHeaderChip)
                root.headerScope = root.headerScope < 0 ? 0 : -1;
            return;
        }
        if (root.headerScope === 0) {
            if (action === "enter") {
                root.headerChipActivated();
                return;
            }
            // Any other grammar leaves the chip and runs on the list.
            root.headerScope = -1;
        }
        // ARC-UI-3: the layout toggle arms on an empty list too (the view
        // swap is independent of the payload).
        if (action === "layout_toggle") {
            root.toggleLayout();
            return;
        }
        if (count === 0)
            return;
        // VI-7: the count register rides the movement (`3j`, `10l`, `5G`).
        var steps = Math.max(1, Backend.keyCount);
        if (action === "page_up" || action === "page_down")
            content.pageJump(action === "page_down");
        else if (action.indexOf("extend_") === 0)
            content.extendSelection("move_" + action.substring(7));
        else if (action === "move_up" || action === "move_down"
                || action === "move_left" || action === "move_right")
            content.moveStep(action, steps);
        else if (action === "top" || action === "bottom")
            content.edgeJump(action === "bottom", steps);
        else if (action === "enter")
            root.activate();
        else if (root.actionHandler)
            root.actionHandler(action);
    }

    // A: title band (20% of the panel by default; less while compressed,
    // UI-57).
    Rectangle {
        radius: Theme.radius
        id: header
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width
        height: root.height * root.effectiveHeaderRatio
        color: "transparent"

        FadeText {
            visible: root.title.length > 0
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: root.width * 0.02
            text: root.title
            color: Palette.onSurface
            font.pixelSize: root.titlePixelSize
            font.bold: true
            animated: false
        }
    }

    // B: the scroll body (the rest), bottom-anchored.
    Rectangle {
        radius: Theme.radius
        id: body
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width
        color: "transparent"
        clip: true

        // The shared scroll body (UI-48 stage, cursor, eased follow, memory).
        ScrollContent {
            id: content
            anchors.fill: parent
            bleed: root.bleed
            model: root.model
            rowDelegate: root.rowDelegate
            selectable: root.selectable
            memoryKey: root.memoryKey
            memoryScope: root.memoryScope
            onItemActivated: (index) => root.itemActivated(index)
            onSelectionChanged: root.selectionChanged()
        }

        FadeText {
            anchors.centerIn: parent
            visible: !root.loading && count === 0 && root.emptyText.length > 0
            text: root.emptyText
            color: Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("body", root.height)
            animated: false
        }

        // UI-22/UI-38: the shared wait state while the first payload lands.
        SkeletonList {
            anchors.fill: parent
            visible: root.loading && count === 0
        }
    }
}
