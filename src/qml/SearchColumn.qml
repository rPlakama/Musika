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
import "ScrollProfiles.js" as ScrollProfiles
import "Theme.js" as Theme

// One column of the floating search: a header (title + count) over the result
// list, with "no matches" and a loading skeleton underneath.
Item {
    id: root

    property string title: ""
    /// The unified search hides the column header; the scope strip carries
    /// the title and count instead.
    property bool showHeader: true
    /// The squeezed-title glyphs (the LeftBar's label → icon rule).
    readonly property string titleIcon: root.title === "Songs"
        ? "qrc:/app/icons/track-svgrepo-com.svg"
        : root.title === "Albums"
            ? "qrc:/app/icons/album.svg"
            : root.title === "Artists"
                ? "qrc:/app/icons/person-walking-svgrepo-com.svg"
                : ""
    property bool roundedArt: false
    property var model: null
    property int cursor: -1
    property bool active: false
    property bool loading: false
    property bool hasQuery: false
    /// ARC-UI-7.13: a search that finishes inside the delay never flashes a
    /// spinner; slower ones fade the loop in instead of spawning it raw.
    property bool _showLoading: false
    Timer {
        id: loadingDelay
        interval: 60
        repeat: false
        running: root.loading && (!root.model || root.model.count === 0)
        onRunningChanged: if (!running) root._showLoading = false
        onTriggered: root._showLoading = true
    }

    signal picked(int index)
    /// ARC-UI-7.9: forwarded from the result body for host badges/status.
    signal selectionChanged

    // ARC-I: which recycled Alt-hold verb set this column arms ("song",
    // "album" or "artist"); the column only relays the cursor row it anchors.
    property string quickActionKind: ""
    /// VI-4: the id field a selection batch resolves to (set by the overlay).
    property string selectionField: "songId"
    signal quickAction(var target, var actions)

    QuickActions {
        id: verbs
    }

    /// ARC-UI-3: the column owns its cursor; the body mirrors it both ways
    /// (it cannot be an alias — the body's view swaps list ⇄ grid). The
    /// mark window follows the rendered rows.
    onCursorChanged: {
        if (content.currentIndex !== cursor)
            content.currentIndex = cursor;
        root.requestMarkWindow();
    }

    Connections {
        target: content
        function onCurrentIndexChanged() {
            if (root.cursor !== content.currentIndex)
                root.cursor = content.currentIndex;
        }
        function onFirstVisibleIndexChanged() { root.requestMarkWindow(); }
        function onLastVisibleIndexChanged() { root.requestMarkWindow(); }
    }

    /// The rows this column can actually display (the search fetch cap).
    readonly property int visibleRowCount: content.visibleRowCount

    /// VI-4: the selection state lives in `ScrollContent` below; the column
    /// owns only its cursor and the id field.
    readonly property bool hasSelection: content.hasSelection
    readonly property int selectionCount: content.selectionCount

    /// VI-4: `v` toggles visual mode on the cursor row.
    function toggleSelection() {
        content.toggleSelection();
    }

    /// VI-4: `V` selects every result (the cursor is this column's binding,
    /// so it moves first, then the anchor widens).
    function selectAll() {
        if (quickActionKind === "" || !model || model.count === 0)
            return;
        root.cursor = model.count - 1;
        content.selectAllRows();
        content.goto(root.cursor);
    }

    function clearSelection() {
        content.clearSelection();
    }

    /// Ask the backend for the download state of the rendered rows only
    /// (album rows included: cached = every track finished).
    function requestMarkWindow(force) {
        if (root.model === null)
            return;
        var songs = [];
        var albums = [];
        var lo = content.firstVisibleIndex;
        var hi = content.lastVisibleIndex;
        if (lo < 0 || hi < lo) {
            lo = Math.max(0, root.cursor);
            hi = Math.min(root.model.count - 1, lo);
        }
        for (var i = lo; i <= hi; ++i) {
            var row = root.model.get(i);
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

    // A fetch finished: re-read this column's own window (forced).
    Connections {
        target: Backend
        function onRowReload() { root.requestMarkWindow(true); }
        // UI-60: patch a rated hit in place.
        function onRatingChanged(kind, id, rating) {
            if (!root.model)
                return;
            for (var i = 0; i < root.model.count; ++i) {
                var row = root.model.get(i);
                if (!row)
                    continue;
                if ((kind === "album" && row.albumId === id)
                        || (kind === "artist" && row.artistId === id))
                    root.model.setProperty(i, "rating", rating);
            }
        }
    }

    /// Instant snap for far jumps (`gg`/`G`): the lists do the same through
    /// `Theme.cursorEdge`, so the view-owned follow has nothing left to
    /// animate once the target is already on screen.
    function snapToCursor() {
        content.goto(root.cursor);
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`).
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow

    function searchRows() {
        return content.searchRows();
    }

    function searchJump(row) {
        if (row < 0 || row >= model.count)
            return;
        root.cursor = row;
        content.goto(row);
    }

    /// `Ctrl-D`/`Ctrl-U` in the active result column.
    function pageJump(down) {
        if (!root.model || root.model.count === 0)
            return;
        content.pageJump(down);
        root.cursor = content.currentIndex;
    }

    /// Shift+Arrow in the active result column.
    function extendSelection(action) {
        if (!root.model || root.model.count === 0)
            return;
        content.extendSelection(action);
        root.cursor = content.currentIndex;
    }

    /// Shift+Click on a result row.
    function extendSelectionTo(index) {
        if (!root.model || root.model.count === 0)
            return;
        content.extendSelectionTo(index);
        root.cursor = content.currentIndex;
    }

    /// ARC-I: Alt armed the gesture — anchor on the cursor row; a visual
    /// selection swaps in the batch verb set (VI-4).
    function armQuickActions() {
        if (quickActionKind === "" || !model || model.count === 0 || cursor < 0)
            return;
        var row = content.view.itemAtIndex(cursor);
        var actions = null;
        if (root.hasSelection) {
            actions = verbs.forSelection(quickActionKind, content.selectionIds(selectionField),
                content.selectionStarred(selectionField));
        } else {
            var item = model.get(cursor);
            var preview = function(level) {
                root.model.setProperty(cursor, "rating", level);
            };
            actions = item
                ? (quickActionKind === "artist" ? verbs.artist(item, preview)
                    : quickActionKind === "album"
                        ? verbs.album(item, Backend.cachedAlbumIds.indexOf(item.albumId) !== -1, preview)
                        : verbs.song(item))
                : null;
        }
        if (!actions)
            return;
        // ARC-I-4: the heart tracks the verb immediately (see `MediaList`).
        if (actions.south && actions.south.invoke) {
            var want = actions.south.favorite === true;
            var star = actions.south.invoke;
            actions.south.invoke = function() {
                star();
                content.markStarred(want);
            };
        }
        if (row)
            quickAction(row, actions);
    }

    // UI-48: the highlight block overflows the row bounds by this much on both
    // sides, filling the column gutters. The overflow stage below clips it at
    // the gutter edge, so it reads as a full-bleed block and never crosses the
    // UI-47 divider.
    readonly property real overflow: width * 0.012

    /// The full column area: the overlay passes the row height so every
    /// section shares its title size.
    property real headerRefHeight: 0
    readonly property real titlePixelSize:
        (root.headerRefHeight > 0 ? root.headerRefHeight : height) * 0.10 * 0.42
    readonly property real headerH: root.showHeader
        ? Math.max(height * 0.10, titlePixelSize * 1.6) : 0
    /// Optional shared row reference: when a host wants rows to scale from a
    /// common height instead of this column's own (0 = this column's height).
    property real rowRefHeight: 0
    /// The stable sizing height for the rows: the shared reference when the
    /// host supplies one, else this column's own height. `listH` derives from
    /// it directly — never from `content.adaptiveRowHeight`, which depends on
    /// the content height `listH` sets (a binding loop).
    readonly property real _rowSizingHeight: root.rowRefHeight > 0
        ? root.rowRefHeight : root.height
    /// The model owns the adaptive row; this column only snaps its viewport
    /// to full rows so the footer never slices one in half.
    readonly property real rowH: content.adaptiveRowFor(root._rowSizingHeight)
    readonly property real listH: root.rowH > 0
        ? Math.max(root.rowH,
            ScrollProfiles.snapBody(root.height - root.headerH, root.rowH, 0))
        : 0
    /// A live query with no result rows: the full body centres the "no" glyph.
    readonly property bool noMatches: root.hasQuery && !root.loading
        && root.model !== null && root.model.count === 0
    /// The body keeps the full snapped row area; an empty column centres the
    /// "no" glyph there instead of collapsing to it.
    readonly property real bodyHeight: root.listH

    Column {
        anchors.fill: parent
        spacing: 0

        Item {
            id: header
            width: parent.width
            height: root.headerH
            visible: root.showHeader

            // The title keeps the room left of the count pills; when that
            // room can no longer hold it, a mapped title trades the word for
            // its segment icon (the LeftBar's rule).
            readonly property real titleRoom: Math.max(0,
                width - countBadge.width - root.headerH * 0.2
                - (selectionBadge.visible
                    ? selectionBadge.width + root.headerH * 0.12 : 0))
            readonly property bool titleFits: titleLabel.implicitWidth <= titleRoom

            FadeText {
                id: titleLabel
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                width: header.titleRoom
                visible: header.titleFits || root.titleIcon.length === 0
                text: root.title
                color: root.active ? Palette.primary : Palette.onSurfaceVariant
                font.pixelSize: root.titlePixelSize
                font.bold: root.active
                elide: Text.ElideRight
                animated: false
            }

            SymbolIcon {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                visible: !header.titleFits && root.titleIcon.length > 0
                width: Math.min(root.headerH * 0.5, header.width * 0.25)
                height: width
                source: root.titleIcon
                symbolColor: root.active ? Palette.primary : Palette.onSurfaceVariant
            }

            // VI-4: the selection count sits beside the result count.
            SelectionBadge {
                id: selectionBadge
                compact: true
                anchors.right: countBadge.left
                anchors.verticalCenter: parent.verticalCenter
                anchors.rightMargin: root.headerH * 0.12
                count: root.selectionCount
                pixelSize: root.headerH * 0.34
            }

            // Count badge: contained by its own pill so the number can never
            // read as the next column's title.
            Rectangle {
                id: countBadge
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                implicitWidth: countLabel.implicitWidth + root.headerH * 0.36
                implicitHeight: root.headerH * 0.52
                radius: Theme.radius
                color: Palette.surfaceContainerHighest
                border.color: root.active ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.headerH * Theme.hairlineRatio)

                FadeText {
                    id: countLabel
                    anchors.centerIn: parent
                    text: root.model ? root.model.count : 0
                    color: root.active ? Palette.primary : Palette.onSurfaceVariant
                    font.pixelSize: root.headerH * 0.34
                    animated: false
                }
            }

            Rectangle {
                radius: Theme.radius
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Math.max(1, parent.height * Theme.hairlineRatio)
                color: root.active ? Palette.primary : Palette.outlineVariant
            }
        }

        Item {
            id: listItem
            width: parent.width
            height: root.bodyHeight

            // UI-48: the shared scroll body carries the overflow stage, so
            // the highlight bleeds past the row (clipped at the gutter edge)
            // while rows still clip vertically at the list bounds.
            ScrollContent {
                id: content
                anchors.fill: parent
                context: "search"
                model: root.model
                selectable: quickActionKind !== ""
                onSelectionChanged: root.selectionChanged()
                // ARC-UI-2 M1: the columns paint the same three cells.
                searchFields: ["title", "subtitle", "meta"]
                // UI-58: rows scale from the shared panel height (not the
                // section left over by the stack). The precomputed extent
                // keeps the shared glide off Qt's lazy estimate.
                rowRefHeight: root._rowSizingHeight

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
                        /// UI-60: 0..=5; album/artist hits carry it.
                        required property int rating

                        readonly property bool current: index === root.cursor
                        // VI-4: the row sits inside the visual selection range.
                        readonly property bool selectedRow: content.selected(index)
                        // UI-45: the focused cursor or any hovered row leads.
                        readonly property bool emphasized: (row.current && root.active) || rowHover.hovered

                        width: ListView.view.width
                        height: content.adaptiveRowHeight
                        // The row is a layout container only (UI-48): the block
                        // highlight lives in its own layer below, so it can
                        // overflow the row without touching the content geometry.
                        color: "transparent"

                        // Dedicated highlight layer over the row block, matching
                        // the main lists and the fullscreen queue (shared
                        // RowHighlight): cursor fill + hover fill, full width.
                        RowHighlight {
                            current: row.current
                            active: root.active
                            hovered: rowHover.hovered
                            selected: row.selectedRow
                            overflow: root.overflow
                        }

                        // The shared list row: the result columns are the
                        // same cover/title/subtitle/meta/marks model as the
                        // library tabs and the queues.
                        RowBody {
                            anchors.fill: parent
                            viewWidth: row.width
                            rowHeight: row.height
                            emphasized: row.emphasized
                            current: row.current && root.active
                            roundedArt: root.roundedArt
                            showCover: true
                            showSubtitle: row.subtitle.length > 0
                            cover: row.cover
                            title: row.title
                            subtitle: row.subtitle
                            meta: row.meta
                            titlePixelSize: Theme.clamp(row.height * 0.3, 12, 18)
                            subtitlePixelSize: Theme.clamp(row.height * 0.24, 10, 14)
                            metaPixelSize: Theme.clamp(row.height * 0.26, 10, 14)
                            titleColor: (row.current && root.active) || row.selectedRow
                                ? Palette.onSecondaryContainer : Palette.onSurface
                            subtitleColor: (row.current && root.active) || row.selectedRow
                                ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                            metaColor: (row.current && root.active) || row.selectedRow
                                ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                            reserveMeta: false
                            metaAlignRight: true
                            leftMargin: 0
                            textGap: row.height * 0.18
                            rightMargin: row.height * 0.18
                            marksMargin: row.height * 0.051
                            markCursor: root.cursor
                            visibleStart: content.firstVisibleIndex
                            visibleEnd: content.lastVisibleIndex
                            markIndex: row.index
                            markSongId: row.songId
                            markAlbumId: row.albumId
                            markStarred: row.starred
                            markRating: row.rating
                            markRatingVisible: root.quickActionKind === "album"
                                || root.quickActionKind === "artist"
                            searchQuery: content.searchQuery
                            searchCurrent: row.index === content.searchCurrentRow
                        }

                        // Hover only signals clickability (S-5); the cursor moves
                        // on click or keys, never on hover.
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
                            onTapped: root.picked(row.index)
                        }
                    }
                }
            }

            SymbolIcon {
                id: noGlyph
                anchors.centerIn: parent
                visible: root.noMatches || noGlyph.opacity > 0
                opacity: root.noMatches ? 1 : 0
                width: Math.max(14, Math.min(listItem.height * 0.7,
                    listItem.width * 0.2))
                height: width
                source: "qrc:/app/icons/no.svg"
                symbolColor: Palette.onSurfaceVariant

                Behavior on opacity {
                    enabled: !Theme.reduceMotion
                    NumberAnimation {
                        duration: 60
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.decelerate
                    }
                }
            }

            // While the request is in flight show the shared fetch loop (the
            // download button's glyph), never the no-result mark. The no.svg
            // only appears once the response landed empty (`noMatches`).
            SymbolIcon {
                id: loadingGlyph
                anchors.centerIn: parent
                visible: root._showLoading
                opacity: root._showLoading ? 1 : 0
                width: Math.max(14, Math.min(listItem.height * 0.7,
                    listItem.width * 0.2))
                height: width
                source: "qrc:/app/icons/loading-loop.svg"
                symbolColor: Palette.onSurfaceVariant

                Behavior on opacity {
                    enabled: !Theme.reduceMotion
                    NumberAnimation {
                        duration: 60
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.decelerate
                    }
                }

                NumberAnimation on rotation {
                    from: 0
                    to: 360
                    duration: Theme.motion.spin
                    loops: Animation.Infinite
                    running: loadingGlyph.visible && !Theme.reduceMotion
                }
            }
        }
    }
}
