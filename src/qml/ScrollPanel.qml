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
Rectangle {
    id: root

    // --- Main rectangle ------------------------------------------------------
    property alias background: root.color

    // --- Split geometry ------------------------------------------------------
    // A = this fraction of the height, B = the rest. Both bands are
    // horizontal-centred; B is bottom-anchored to the surface.
    property real headerRatio: 0.2
    // UI-48: horizontal gutter the body overflows into (highlight bleed).
    property real bleed: 0

    // UI-57: compressed viewports (the same `narrow` trigger ArtistEntry
    // uses) ramp the title toward the headline floor and shrink the band, so
    // the reclaimed height goes to the list. `compression` is continuous in
    // width (floor 0.5), so crossing the breakpoint never jumps; full-size
    // viewports are untouched (compression = 1).
    readonly property bool compressed: Theme.isNarrow(root.width)
    readonly property real compression: Math.max(0.5,
        Math.min(1, root.width / Theme.breakpoints.narrow))
    readonly property real titlePixelSize: {
        // The linear-toward-a-clamp ramp ArtistEntry's description uses; the
        // 18 px floor is the headline token (never body-text weight).
        var full = Theme.fontSize("headline", root.height);
        return Theme.clamp(18 + (full - 18) * root.compression, 18, full);
    }
    readonly property real effectiveHeaderRatio: root.headerRatio
        * (0.7 + 0.3 * root.compression)

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
    property alias count: content.count
    /// Escape hatch for surfaces that need the ListView itself (lyrics follow).
    property alias view: content.view
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
        ensureVisible();
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

    function ensureVisible(index) {
        content.ensureVisible(index);
    }

    function activate() {
        content.activate();
    }

    // Shared cursor grammar: j/k, gg/G, Enter; unknown actions go to
    // `actionHandler` (search columns, queue reorder, …). Cursor moves are
    // followed by the view itself (ScrollContent highlight machinery); gg/G
    // snap instantly via `cursorEdge`.
    function handleAction(action) {
        if (count === 0)
            return;
        if (action === "move_up")
            currentIndex = Math.max(0, currentIndex - 1);
        else if (action === "move_down")
            currentIndex = Math.min(count - 1, currentIndex + 1);
        else if (action === "top" || action === "bottom")
            Theme.cursorEdge(content.view, action === "bottom", ListView.Contain);
        else if (action === "enter")
            root.activate();
        else if (root.actionHandler)
            root.actionHandler(action);
    }

    // A: title band (20% of the panel by default; less while compressed,
    // UI-57).
    Rectangle {
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
