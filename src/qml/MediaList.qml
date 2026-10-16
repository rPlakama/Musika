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

// Full-width library list shared by the Albums, Songs and Artists segments:
// the shared `ScrollPanel` shape (title band over the scroll body) carrying
// covers, titles and right-aligned meta. The wrappers (`AlbumList`,
// `SongList`, `ArtistList`) own the backend channel and map their payload
// into rows via `setRows`; this component owns the rows and row rendering.
ScrollPanel {
    id: root

    // Current order label beside the Shift-H / Shift-L cue; an empty label
    // hides the cue (e.g. Artists, whose order is fixed for now).
    property string orderText: ""
    // Artists wear a circle avatar (the one sanctioned circle, R3).
    property bool roundedArt: false

    signal activated(var item)

    // ARC-UI-1: which recycled Alt-hold verb set this list arms — "song" for
    // track rows, "album" for album rows, "" for no overlay. The verbs live
    // in `QuickActions`; the list only relays the row it anchors.
    property string quickActionKind: ""
    signal quickAction(var target, var actions)

    QuickActions {
        id: verbs
    }

    // The row download marks read the global ledger set; make sure it is
    // loaded even when the fullscreen player was never opened.
    Component.onCompleted: Backend.requestDownloads()

    /// VI-4: the selection state lives in `ScrollContent` and is forwarded by
    /// `ScrollPanel` (`toggleSelection`/`selectAll`/`clearSelection`/…); the
    /// list only gates it on the Alt verb kind and names the id field batch
    /// actions resolve to.
    selectable: quickActionKind !== ""
    readonly property string selectionField: quickActionKind === "album" ? "albumId" : "songId"

    /// ARC-43: Alt armed the gesture — anchor on the keyboard cursor row, or
    /// on the range with the selection verb set (VI-4).
    function armQuickActions() {
        if (quickActionKind === "" || count === 0 || currentIndex < 0)
            return;
        var row = view.itemAtIndex(currentIndex);
        var actions = null;
        if (root.hasSelection) {
            actions = verbs.forSelection(quickActionKind, root.selectedIds(selectionField),
                root.selectedStarred(selectionField));
        } else {
            var item = mediaRows.get(currentIndex);
            actions = item
                ? (quickActionKind === "album" ? verbs.album(item) : verbs.song(item))
                : null;
        }
        if (!actions)
            return;
        // ARC-I-4: the heart tracks the verb immediately — the affected model
        // row(s) are written in place (cursor or selection), no reload.
        if (actions.south && actions.south.invoke) {
            var want = actions.south.favorite === true;
            var star = actions.south.invoke;
            actions.south.invoke = function() {
                star();
                root.markStarred(want);
            };
        }
        if (row)
            quickAction(row, actions);
    }

    emptyText: "Nothing indexed yet — run a sync"

    ListModel {
        id: mediaRows
    }

    model: mediaRows
    // ARC-UI-2 M1: the rows paint title, subtitle and meta; nothing else.
    searchFields: ["title", "subtitle", "meta"]
    // UI-58: the row fraction the adaptive rule preserves.
    rowRatio: 0.105
    // ARC-UI-3: Shift-C flips every segment between the row list and a cover
    // wall — the same shared body ArtistEntry uses, with gg/G, the search
    // contract, selection and the scroll memory.
    layouts: ["list", "grid"]
    columns: 4
    cellHeight: root.cellWidth * 0.78 + root.height * 0.085

    function setRows(rows) {
        // VI-4: a reload invalidates the selected range.
        root.clearSelection();
        mediaRows.clear();
        for (var i = 0; i < rows.length; i++)
            mediaRows.append(rows[i]);
        // UI-10: a filter/order swap lands on row 0 with the top pinned —
        // the old cursor/offset point at different rows now, so the restore
        // would chase a stale spot.
        if (root.resetPending) {
            root.resetPending = false;
            root.resetViewNow();
            return;
        }
        // UI-54: the first payload after a reopen restores the saved viewport
        // (cursor + offset), so the highlight sits where the user left it;
        // later reloads keep the live state.
        if (root.restore())
            return;
        // UI-10: a cursor row exists from the first paint.
        if (root.count > 0 && (root.currentIndex < 0 || root.currentIndex >= root.count))
            root.currentIndex = 0;
    }

    onItemActivated: (index) => {
        // VI-4: opening/playing a row ends the visual selection.
        root.clearSelection();
        var item = mediaRows.get(index);
        if (item)
            root.activated(item);
    }

    // VI-4: the selection count pill rides at the header's right edge.
    SelectionBadge {
        id: selectionBadge
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: root.width * 0.02
        count: root.selectionCount
        pixelSize: root.height * 0.021
    }

    // Header (A): the Shift-H / Shift-L order cue, vertically centred.
    Row {
        visible: root.orderText.length > 0
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        // VI-4: step left of the selection pill when it is up.
        anchors.rightMargin: root.width * 0.02
            + (selectionBadge.visible ? selectionBadge.width + root.height * 0.012 : 0)
        spacing: root.height * 0.012

        FadeText {
            anchors.verticalCenter: parent.verticalCenter
            text: "Shift-H / Shift-L"
            color: Palette.onSurfaceVariant
            font.pixelSize: root.height * 0.021
        }

        FadeText {
            anchors.verticalCenter: parent.verticalCenter
            text: "/"
            color: Palette.onSurfaceVariant
            font.pixelSize: root.height * 0.021
        }

        FadeText {
            anchors.verticalCenter: parent.verticalCenter
            text: root.orderText
            color: Palette.onSurfaceVariant
            font.pixelSize: root.height * 0.021
        }
    }

    // Body (B): one row per payload item.
    rowDelegate: Component {
        Rectangle {
            id: row
            required property int index
            required property string title
            required property string subtitle
            required property string meta
            required property string cover
            required property string songId
            required property bool starred

            readonly property real viewWidth: ListView.view ? ListView.view.width : 0
            // Mouse/keyboard parity: the hovered row expands like the cursor.
            readonly property bool emphasized: ListView.isCurrentItem || rowHover.hovered
            // VI-4: the row sits inside the visual selection range.
            readonly property bool selectedRow: root.isSelected(index)

            width: viewWidth
            // UI-58: adaptive — the 18% band scales, past it a row is cut.
            height: root.adaptiveRowHeight > 0
                ? root.adaptiveRowHeight
                : (ListView.view ? ListView.view.height * 0.105 : 0)
            color: (ListView.isCurrentItem || row.selectedRow)
                ? Palette.secondaryContainer
                : (rowHover.hovered
                    ? Palette.surfaceContainerHigh
                    : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest))

            Behavior on color {
                ColorAnimation {
                    duration: Theme.motion.quick
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }

            Rectangle {
                id: cover
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: row.viewWidth * 0.04
                width: row.height * 0.78
                height: row.height * 0.78
                radius: root.roundedArt ? width * 0.5 : Theme.radius
                color: Palette.surfaceContainerHigh
                clip: true
                // UI-45: the art grows with the emphasized row (same
                // animation as the title, transform only).
                scale: (row.emphasized && !Theme.reduceMotion) ? Theme.selectionEmphasis : 1.0

                Behavior on scale {
                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                }

                CoverImage {
                    anchors.fill: parent
                    source: row.cover
                    // ARC-40: ×2 headroom for the UI-45 hover scale.
                    sizeFactor: 2
                }
            }

            Column {
                id: textColumn
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: cover.right
                anchors.leftMargin: row.viewWidth * 0.04
                anchors.right: metaLabel.left
                anchors.rightMargin: row.viewWidth * 0.04
                spacing: row.height * 0.05

                SearchText {
                    id: titleText
                    width: parent.width
                    text: row.title
                    color: Palette.onSurface
                    font.pixelSize: row.height * 0.26
                    elide: Text.ElideRight
                    animated: false
                    // ARC-UI-2 M2: hlsearch blocks from the live `/` session.
                    query: root.searchQuery
                    currentMatch: row.index === root.searchCurrentRow
                    transformOrigin: Item.Left
                    // UI-45 selection stage: the cursor/hover row leads its
                    // neighbours (transform only, no reflow), capped to the
                    // title box so it never reaches the meta gutter.
                    scale: (row.emphasized && !Theme.reduceMotion)
                        ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                        : 1.0

                    Behavior on scale {
                        NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                    }
                }
                SearchText {
                    width: parent.width
                    text: row.subtitle
                    color: Palette.onSurfaceVariant
                    font.pixelSize: row.height * 0.19
                    elide: Text.ElideRight
                    animated: false
                    query: root.searchQuery
                    currentMatch: row.index === root.searchCurrentRow
                }
            }

            SearchText {
                id: metaLabel
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: row.viewWidth * 0.04
                text: row.meta
                color: Palette.onSurfaceVariant
                font.pixelSize: row.height * 0.19
                animated: false
                query: root.searchQuery
                currentMatch: row.index === root.searchCurrentRow
            }

            // UI-56: the state marks lead the meta text — download (the
            // global ledger set) then the favourite heart; none draws
            // anything when absent, so no gaps appear.
            Row {
                id: stateMarks
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: metaLabel.left
                anchors.rightMargin: row.viewWidth * 0.015
                spacing: row.viewWidth * 0.008

                Rectangle {
                    visible: Backend.downloadedIds.indexOf(row.songId) !== -1
                    width: metaLabel.font.pixelSize * 0.96
                    height: width
                    color: "transparent"

                    Icon {
                        anchors.centerIn: parent
                        width: parent.width
                        height: width
                        name: "baseline-download"
                        symbolColor: Palette.tertiary
                    }
                }

                Rectangle {
                    visible: row.starred
                    width: metaLabel.font.pixelSize * 0.8
                    height: width
                    color: "transparent"

                    Icon {
                        anchors.centerIn: parent
                        width: parent.width
                        height: width
                        name: "baseline-favorite"
                        symbolColor: Theme.iconStrong
                    }
                }
            }

            HoverHandler {
                id: rowHover
                cursorShape: Qt.PointingHandCursor
            }

            TapHandler {
                onTapped: {
                    root.currentIndex = row.index;
                    root.activate();
                }
            }
        }
    }

    // The cover-wall cell: the row payload laid out like ArtistEntry's cards
    // (cover with the favourite mark in the corner, title + mixed meta line).
    cellDelegate: Component {
        Rectangle {
            id: cell
            required property int index
            required property string title
            required property string subtitle
            required property string meta
            required property string cover
            required property string songId
            required property bool starred

            readonly property bool current: GridView.isCurrentItem
            readonly property bool selectedCard: root.isSelected(cell.index)
            // UI-50: cursor or hover expands the cover + title.
            readonly property bool emphasized: (cell.current || cellHover.hovered) && !Theme.reduceMotion

            width: GridView.view.cellWidth
            height: GridView.view.cellHeight
            color: "transparent"

            Rectangle {
                anchors.fill: parent
                anchors.margins: cell.width * 0.03
                radius: Theme.radius
                color: (cell.current || cell.selectedCard)
                    ? Palette.secondaryContainer
                    : (cellHover.hovered ? Palette.surfaceContainerHigh : "transparent")

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }
            }

            Rectangle {
                id: coverBox
                width: cell.width * 0.78
                height: width
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: parent.top
                anchors.topMargin: cell.height * 0.05
                radius: root.roundedArt ? width * 0.5 : Theme.radius
                color: Palette.surfaceContainerLowest
                border.color: cell.current ? Palette.primary : "transparent"
                border.width: Math.max(1, cell.height * Theme.hairlineRatio)
                clip: true

                transformOrigin: Item.Center
                scale: cell.emphasized ? Theme.selectionEmphasis : 1.0

                Behavior on scale {
                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                }

                CoverImage {
                    anchors.fill: parent
                    source: cell.cover
                    // ARC-40: ×2 headroom for the UI-50 hover scale.
                    sizeFactor: 2
                }

                // UI-56: the favourite mark rides the cover corner; the
                // download mark (the global ledger set) mirrors it on the
                // other corner.
                Icon {
                    visible: cell.starred
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: parent.width * 0.06
                    width: parent.width * 0.085
                    height: width
                    name: "baseline-favorite"
                    symbolColor: Theme.iconStrong
                }

                Icon {
                    visible: Backend.downloadedIds.indexOf(cell.songId) !== -1
                    anchors.left: parent.left
                    anchors.bottom: parent.bottom
                    anchors.margins: parent.width * 0.06
                    width: parent.width * 0.085 * 1.2
                    height: width
                    name: "baseline-download"
                    symbolColor: Palette.tertiary
                }
            }

            Column {
                anchors.top: coverBox.bottom
                anchors.topMargin: cell.height * 0.02
                anchors.horizontalCenter: parent.horizontalCenter
                width: parent.width * 0.86
                spacing: cell.height * 0.012

                SearchText {
                    id: cellTitle
                    width: parent.width
                    text: cell.title
                    color: Palette.onSurface
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    font.pixelSize: Math.max(10, cell.height * 0.05)
                    animated: false
                    query: root.searchQuery
                    currentMatch: cell.index === root.searchCurrentRow
                    transformOrigin: Item.Center
                    scale: cell.emphasized
                        ? Theme.fitScale(cellTitle.width, cellTitle.paintedWidth, Theme.selectionEmphasis)
                        : 1.0

                    Behavior on scale {
                        NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                    }
                }

                // One muted line: the row's subtitle and meta joined, so a
                // song reads "Artist · Album · 3:45" under its cover.
                SearchText {
                    width: parent.width
                    text: cell.subtitle
                        + (cell.subtitle.length > 0 && cell.meta.length > 0 ? "   ·   " : "")
                        + cell.meta
                    color: Palette.onSurfaceVariant
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    font.pixelSize: Math.max(9, cell.height * 0.038)
                    animated: false
                    query: root.searchQuery
                    currentMatch: cell.index === root.searchCurrentRow
                }
            }

            HoverHandler {
                id: cellHover
                cursorShape: Qt.PointingHandCursor
            }

            TapHandler {
                onTapped: {
                    root.currentIndex = cell.index;
                    root.activate();
                }
            }
        }
    }
}
