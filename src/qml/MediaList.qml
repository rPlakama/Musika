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
// into rows via `setRows`; this component owns the rows, row rendering and
// the one lazy-load state machine.

ScrollPanel {
    id: root

    RowMetrics {
        id: rowMetrics
        viewWidth: root.width
        rowHeight: root.adaptiveRowHeight
    }

    // The wrappers hook their backend channel through these two virtual
    // functions; the base versions keep a wrapper without a loader inert.
    function reload() {
        root.requestFull(true);
    }

    function requestPreload() {
    }

    function requestFull(showLoading) {
    }

    // One lazy-load state machine for every segment: a scope switch either
    // preloads (warming) or activates the tab (the full payload).
    property bool _preloaded: false
    property bool _fullLoaded: false
    property bool _preloadOnly: false

    function start() {
        if (root._preloadOnly)
            root.preload();
        else
            root.activateTab();
    }

    function activateTab() {
        root.stopPreload();
        if (root._fullLoaded)
            return;
        root.requestFull(root.count === 0);
    }

    function preload() {
        // Never shrink an already-loaded model back to a warm head.
        if (root._preloaded || root._fullLoaded)
            return;
        root._preloaded = true;
        root.startPreloadTimer();
    }

    Timer {
        id: preloadTimer
        interval: 350
        repeat: false
        onTriggered: root.requestPreload()
    }

    // Expose helper functions so derived components can control the timer
    // without reaching for the base id.
    function stopPreload() {
        preloadTimer.stop();
    }

    function startPreloadTimer() {
        preloadTimer.restart();
    }
    // Current order label beside the Shift-H / Shift-L cue; an empty label
    // hides the cue (e.g. Artists, whose order is fixed for now).
    property string orderText: ""
    // Artists wear a circle avatar (the one sanctioned circle, R3).
    property bool roundedArt: false

    signal activated(var item)
    /// PATH-3: pages append on demand; hosts use this to fetch the next one.
    signal cursorMoved(int index)

    // ARC-UI-1: which recycled Alt-hold verb set this list arms — "song" for
    // track rows, "album" for album rows, "" for no overlay. The inherited
    // `ScrollPanel.quickActionKind` alias reaches the body, which arms; the
    // list only relays its quickAction signal.
    /// UI-60: only album/artist rows own a rating; song rows show the heart
    /// alone (no number-0 badge).
    readonly property bool ratingMarks: quickActionKind === "album" || quickActionKind === "artist"
    // The download window follows the row kind the segment renders.
    markSongs: root.quickActionKind === "song"
    markAlbums: root.quickActionKind !== "song"

    /// The shuffle source for this tab ("" = no chip, no Tab stop).
    readonly property string shuffleKind: root.quickActionKind === "song" ? "songs" : (root.quickActionKind === "album" ? "albums" : "")
    hasHeaderChip: root.shuffleKind.length > 0
    onHeaderChipActivated: Backend.toggleShuffle(root.shuffleKind)

    /// An order/type swap reload: UI-10's cursor-position reset rides the
    /// shared row reload — reset first (the same indices hold different
    /// rows now), re-request the payload, then the global row hook
    /// (marks, hearts, download window) in one entry point.
    function resetRowReload() {
        root.resetView();
        root.reload();
        Backend.TriggerRowReload();
    }

    onCurrentIndexChanged: root.cursorMoved(root.currentIndex)

    /// VI-4: the selection state lives in `ScrollContent` and is forwarded by
    /// `ScrollPanel` (`toggleSelection`/`selectAll`/`clearSelection`/…); the
    /// list only gates it on the Alt verb kind and names the yank kind.
    selectable: quickActionKind !== ""
    /// VI-8a: the shared yank verb reads the same row kind.
    yankKind: quickActionKind

    emptyText: "Nothing indexed yet — run a sync"

    ListModel {
        id: mediaRows
    }

    model: mediaRows
    titleHint: "(Shift-J/K) cycle filter"
    // ARC-UI-2 M1: the rows paint title, subtitle and meta; nothing else.
    searchFields: ["title", "subtitle", "meta"]
    // UI-58: the row fraction the adaptive rule preserves. Size from the
    // panel's body room, not from the body itself, so snapping it cannot
    // feed back into the row unit.
    context: "library"
    rowRefHeight: root.bodyAvail
    /// PATH: fill the model in small event-loop batches, so a large payload
    /// (the album library) cannot block input or the compositor.
    readonly property int fillBatch: 64
    property int _fillGeneration: 0
    property var _fillRows: null
    property int _fillStart: 0

    Timer {
        id: fillTimer
        interval: 0
        repeat: false
        onTriggered: root._appendFill(root._fillRows, root._fillStart, root._fillGeneration)
    }

    function setRows(rows) {
        // VI-4: a reload invalidates the selected range.
        root.clearSelection();
        mediaRows.clear();
        root.loading = true;
        root._fillGeneration++;
        root._fillRows = rows;
        root._fillStart = 0;
        fillTimer.stop();
        root._appendFill(rows, 0, root._fillGeneration);
    }

    function _appendFill(rows, start, generation) {
        if (generation !== root._fillGeneration)
            return;
        var end = Math.min(rows.length, start + root.fillBatch);
        for (var i = start; i < end; i++)
            mediaRows.append(rows[i]);
        if (end < rows.length) {
            root._fillRows = rows;
            root._fillStart = end;
            fillTimer.restart();
            return;
        }
        root._fillRows = null;
        root.loading = false;
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

    /// PATH-3: append one page without clearing the model, the selection or
    /// the live cursor. Mark windows follow automatically.
    function appendRows(rows) {
        for (var i = 0; i < rows.length; i++)
            mediaRows.append(rows[i]);
    }

    onItemActivated: index => {
        // VI-4: opening/playing a row ends the visual selection.
        root.clearSelection();
        var item = mediaRows.get(index);
        if (item)
            root.activated(item);
    }

    /// One list-level pointer handler owns row clicks.
    onRowTapped: (index, shift) => {
        if (shift) {
            root.extendSelectionTo(index);
            return;
        }
        root.currentIndex = index;
        root.activate();
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

    // Header (A): the shuffle toggle (Album/Songs tabs) then the
    // Shift-H / Shift-L order cue, vertically centred —
    // <Title> <gap> <Shuffle> <keybind> <order>.
    ShuffleChip {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: orderRow.left
        anchors.rightMargin: root.height * 0.014
        unit: root.height * 0.03
        kind: root.shuffleKind
        focused: root.headerScope === 0
    }

    Row {
        id: orderRow
        visible: root.orderText.length > 0
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        // VI-4: step left of the selection pill when it is up.
        anchors.rightMargin: root.width * 0.02 + (selectionBadge.visible ? selectionBadge.width + root.height * 0.012 : 0)
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
            radius: Theme.radius
            required property int index
            required property string title
            required property string subtitle
            required property string meta
            required property string cover
            required property string songId
            required property string albumId
            required property bool starred
            /// UI-60: 0..=5; album/artist rows carry it, songs stay 0.
            required property int rating

            readonly property real viewWidth: ListView.view ? ListView.view.width : 0
            // Attached `ListView.isCurrentItem` only resolves on the delegate
            // root; children must read this property (a child would always
            // report false).
            readonly property bool currentItem: ListView.isCurrentItem
            // Mouse/keyboard parity: the hovered row expands like the cursor.
            readonly property bool emphasized: row.currentItem || row.index === root.hoveredIndex
            // VI-4: the row sits inside the visual selection range.
            readonly property bool selectedRow: root.isSelected(index)

            width: viewWidth
            // Selection scale paints above the neighbouring rows instead of
            // letting the row above clip it.
            z: row.emphasized ? 1 : 0
            // UI-58: adaptive — stops at the 18% floor instead of stepping.
            height: root.adaptiveRowHeight > 0 ? root.adaptiveRowHeight : (ListView.view ? ListView.view.height * 0.105 : 0)
            color: "transparent"

            // The shared fill layer: zebra, hover, cursor and visual range.
            RowHighlight {
                index: row.index
                striped: true
                baseColor: Palette.surface
                current: row.currentItem
                hovered: row.index === root.hoveredIndex
                selected: row.selectedRow
            }

            RowBody {
                anchors.fill: parent
                viewWidth: row.viewWidth
                rowHeight: row.height
                metrics: rowMetrics
                emphasized: row.emphasized
                current: row.currentItem
                roundedArt: root.roundedArt
                cover: row.cover
                title: row.title
                subtitle: row.subtitle
                meta: row.meta
                reserveMeta: root.quickActionKind === "song"
                metaAlignRight: root.quickActionKind === "song"
                markCursor: root.currentIndex
                visibleStart: root.firstVisibleIndex
                visibleEnd: root.lastVisibleIndex
                markIndex: row.index
                markSongId: row.songId
                markAlbumId: row.albumId
                markStarred: row.starred
                markRating: row.rating
                markRatingVisible: root.ratingMarks
                searchQuery: root.searchQuery
                searchCurrent: row.index === root.searchCurrentRow
            }
        }
    }

}
