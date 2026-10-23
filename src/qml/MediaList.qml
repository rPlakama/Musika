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
    /// PATH-3: pages append on demand; hosts use this to fetch the next one.
    signal cursorMoved(int index)

    // ARC-UI-1: which recycled Alt-hold verb set this list arms — "song" for
    // track rows, "album" for album rows, "" for no overlay. The verbs live
    // in `QuickActions`; the list only relays the row it anchors.
    property string quickActionKind: ""
    signal quickAction(var target, var actions)
    /// UI-60: only album/artist rows own a rating; song rows show the heart
    /// alone (no number-0 badge).
    readonly property bool ratingMarks: quickActionKind === "album"
        || quickActionKind === "artist"

    QuickActions {
        id: verbs
    }

    /// The shuffle source for this tab ("" = no chip, no Tab stop).
    readonly property string shuffleKind: root.quickActionKind === "song"
        ? "songs"
        : (root.quickActionKind === "album" ? "albums" : "")
    hasHeaderChip: root.shuffleKind.length > 0
    onHeaderChipActivated: Backend.toggleShuffle(root.shuffleKind)

    /// The state marks (heart/download) render only for the rendered band —
    /// the viewport plus the scroll body's overscan.
    function markVisible(index) {
        return root.firstVisibleIndex >= 0
            && index >= root.firstVisibleIndex
            && index <= root.lastVisibleIndex;
    }

    /// Ask the backend for the download state of the rendered rows only —
    /// never the whole ledger. Album rows ride the same call: the backend
    /// reports an album as cached when every track of it is finished.
    function requestMarkWindow(force) {
        var songs = [];
        var albums = [];
        var lo = root.firstVisibleIndex;
        var hi = root.lastVisibleIndex;
        if (lo < 0 || hi < lo) {
            lo = Math.max(0, root.currentIndex);
            hi = Math.min(mediaRows.count - 1, lo);
        }
        for (var i = lo; i <= hi; ++i) {
            var row = mediaRows.get(i);
            if (!row)
                continue;
            if (row.songId)
                songs.push(row.songId);
            else if (row.albumId)
                albums.push(row.albumId);
        }
        if (force)
            Backend.refreshDownloads(JSON.stringify(songs), JSON.stringify(albums));
        else
            Backend.requestDownloads(JSON.stringify(songs), JSON.stringify(albums));
    }

    onFirstVisibleIndexChanged: root.requestMarkWindow()
    onLastVisibleIndexChanged: root.requestMarkWindow()

    // A fetch finished: re-read this surface's own window (forced — the
    // dedupe must not swallow the update).
    Connections {
        target: Backend
        function onRowReload() { root.requestMarkWindow(true); }
        // UI-60: patch a rated album/artist row in place.
        function onRatingChanged(kind, id, rating) {
            for (var i = 0; i < mediaRows.count; ++i) {
                var row = mediaRows.get(i);
                if (!row)
                    continue;
                if ((kind === "album" && row.albumId === id)
                        || (kind === "artist" && row.artistId === id))
                    mediaRows.setProperty(i, "rating", rating);
            }
        }
    }

    /// A row's download mark: its song is cached, or it is an album row
    /// whose every track is.
    function markCached(songId, albumId) {
        return (songId.length > 0 && Backend.downloadedIds.indexOf(songId) !== -1)
            || (albumId.length > 0 && Backend.cachedAlbumIds.indexOf(albumId) !== -1);
    }

    /// An order/type swap reload: UI-10's cursor-position reset rides the
    /// shared row reload — reset first (the same indices hold different
    /// rows now), re-request the payload, then the global row hook
    /// (marks, hearts, download window) in one entry point.
    function resetRowReload() {
        root.resetView();
        root.reload();
        Backend.TriggerRowReload();
    }

    Component.onCompleted: root.requestMarkWindow()
    onCurrentIndexChanged: {
        root.requestMarkWindow();
        root.cursorMoved(root.currentIndex);
    }

    /// VI-4: the selection state lives in `ScrollContent` and is forwarded by
    /// `ScrollPanel` (`toggleSelection`/`selectAll`/`clearSelection`/…); the
    /// list only gates it on the Alt verb kind and names the id field batch
    /// actions resolve to.
    selectable: quickActionKind !== ""
    readonly property string selectionField: quickActionKind === "album"
        ? "albumId"
        : (quickActionKind === "artist" ? "artistId" : "songId")

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
            // UI-60: the overlay's rating step repaints the row from the
            // local model immediately (persistence is separate).
            var preview = function(level) {
                mediaRows.setProperty(currentIndex, "rating", level);
            };
            actions = item
                ? (quickActionKind === "album"
                    ? verbs.album(item, Backend.cachedAlbumIds.indexOf(item.albumId) !== -1, preview)
                    : (quickActionKind === "artist"
                        ? verbs.artist(item, preview)
                        : verbs.song(item)))
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
    titleHint: "(Shift-J/K) cycle filter"
    // ARC-UI-2 M1: the rows paint title, subtitle and meta; nothing else.
    searchFields: ["title", "subtitle", "meta"]
    // UI-58: the row fraction the adaptive rule preserves. Size from the
    // panel's body room, not from the body itself, so snapping it cannot
    // feed back into the row unit.
    context: "library"
    rowRefHeight: root.bodyAvail
    // ARC-UI-3: Shift-C flips every segment between the row list and a cover
    // wall — the same shared body ArtistEntry uses, with gg/G, the search
    // contract, selection and the scroll memory.
    layouts: ["list", "grid"]

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
        onTriggered: root._appendFill(root._fillRows, root._fillStart,
            root._fillGeneration)
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
        root.requestMarkWindow();
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
        root.requestMarkWindow();
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
            radius: Theme.radius
            id: row
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
            // Mouse/keyboard parity: the hovered row expands like the cursor.
            readonly property bool emphasized: ListView.isCurrentItem || rowHover.hovered
            // VI-4: the row sits inside the visual selection range.
            readonly property bool selectedRow: root.isSelected(index)

            width: viewWidth
            // UI-58: adaptive — stops at the 18% floor instead of stepping.
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

            RowBody {
                anchors.fill: parent
                viewWidth: row.viewWidth
                rowHeight: row.height
                emphasized: row.emphasized
                current: ListView.isCurrentItem
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

            HoverHandler {
                id: rowHover
                cursorShape: Qt.PointingHandCursor
            }

            TapHandler {
                acceptedModifiers: Qt.ShiftModifier
                onTapped: root.extendSelectionTo(row.index)
            }
            TapHandler {
                acceptedModifiers: Qt.NoModifier
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
            radius: Theme.radius
            id: cell
            required property int index
            required property string title
            required property string subtitle
            required property string meta
            required property string cover
            required property string songId
            required property string albumId
            required property bool starred
            /// UI-60: 0..=5; album cells carry it, songs stay 0.
            required property int rating

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
                    // ARC-40: ×1.25 headroom for the UI-50 hover scale.
                    sizeFactor: 1.25
                }

                // UI-56/UI-60: the favourite mark rides the cover corner with
                // the rating count at its bottom-right; the download mark
                // (the global ledger set) mirrors it on the other corner.
                Item {
                    visible: root.markVisible(cell.index)
                        && (cell.starred
                            || (root.ratingMarks && cell.rating > 0))
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    anchors.margins: parent.width * 0.06
                    width: parent.width * 0.085
                    height: width

                    Icon {
                        anchors.fill: parent
                        visible: cell.starred
                        name: "baseline-favorite"
                        symbolColor: Theme.iconStrong
                    }

                    RatingBadge {
                        visible: root.ratingMarks
                        anchors.right: parent.right
                        anchors.bottom: parent.bottom
                        width: parent.width * 0.84
                        height: width
                        rating: cell.rating
                    }
                }

                Icon {
                    visible: root.markVisible(cell.index) && root.markCached(cell.songId, cell.albumId)
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
                acceptedModifiers: Qt.ShiftModifier
                onTapped: root.extendSelectionTo(cell.index)
            }
            TapHandler {
                acceptedModifiers: Qt.NoModifier
                onTapped: {
                    root.currentIndex = cell.index;
                    root.activate();
                }
            }
        }
    }
}
