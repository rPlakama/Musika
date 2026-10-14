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
    property alias currentIndex: list.currentIndex
    property alias count: list.count
    /// Escape hatch for surfaces that need the ListView itself (lyrics follow).
    property alias view: list
    property string memoryKey: ""
    property int memoryScope: -1
    property bool _restored: false
    /// Optional handler for extra keyboard actions; returning nothing is fine.
    property var actionHandler: null

    signal itemActivated(int index)

    color: Palette.surface

    // UI-41/UI-54: the shared eased-scroll + scroll-memory seam.
    ListScroll {
        id: scroll
    }

    // UI-54: keep the memory current while the panel lives.
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

    /// UI-54: first payload after a reopen restores cursor + offset; later
    /// reloads keep the live state. Returns true when a memory was applied.
    function restore() {
        if (root._restored)
            return false;
        root._restored = true;
        return scroll.recall(root.memoryKey, list);
    }

    function ensureVisible(index) {
        scroll.ensureVisible(list, index === undefined ? list.currentIndex : index);
    }

    function activate() {
        if (list.currentIndex >= 0)
            root.itemActivated(list.currentIndex);
    }

    // Shared cursor grammar: j/k, gg/G, Enter; unknown actions go to
    // `actionHandler` (search columns, queue reorder, …). Every move keeps
    // the cursor visible with the eased follow.
    function handleAction(action) {
        if (list.count === 0)
            return;
        if (action === "move_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down")
            list.currentIndex = Math.min(list.count - 1, list.currentIndex + 1);
        else if (action === "top" || action === "bottom")
            Theme.cursorEdge(list, action === "bottom", ListView.Contain);
        else if (action === "enter")
            root.activate();
        else if (root.actionHandler)
            root.actionHandler(action);
        root.ensureVisible();
    }

    // A: title band (20% of the panel by default).
    Rectangle {
        id: header
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width
        height: root.height * root.headerRatio
        color: "transparent"

        FadeText {
            visible: root.title.length > 0
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: root.width * 0.02
            text: root.title
            color: Palette.onSurface
            font.pixelSize: Theme.fontSize("headline", root.height)
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

        // UI-48: the overflow stage is wider than the list and carries the
        // clip, so a row highlight can bleed into the gutters (`bleed`).
        Item {
            id: stage
            x: -root.bleed
            width: parent.width + 2 * root.bleed
            height: parent.height
            clip: true

            ListView {
                id: list
                anchors.fill: parent
                anchors.leftMargin: root.bleed
                anchors.rightMargin: root.bleed
                // With bleed the stage clips for it; otherwise the list does.
                clip: root.bleed <= 0
                model: root.model
                boundsBehavior: Flickable.StopAtBounds
                delegate: root.rowDelegate
            }
        }

        FadeText {
            anchors.centerIn: parent
            visible: !root.loading && list.count === 0 && root.emptyText.length > 0
            text: root.emptyText
            color: Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("body", root.height)
            animated: false
        }

        // UI-22/UI-38: the shared wait state while the first payload lands.
        SkeletonList {
            anchors.fill: parent
            visible: root.loading && list.count === 0
        }
    }
}
