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

// One column of the floating search: a header (title + count) over the result
// list, with "no matches" and a loading skeleton underneath.
Item {
    id: root

    property string title: ""
    property bool roundedArt: false
    property var model: null
    property int cursor: -1
    property bool active: false
    property bool loading: false
    property bool hasQuery: false

    signal picked(int index)

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
    /// (it cannot be an alias — the body's view swaps list ⇄ grid).
    onCursorChanged: if (content.currentIndex !== cursor) content.currentIndex = cursor

    Connections {
        target: content
        function onCurrentIndexChanged() {
            if (root.cursor !== content.currentIndex)
                root.cursor = content.currentIndex;
        }
    }

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
            actions = item
                ? (quickActionKind === "artist" ? verbs.artist(item)
                    : quickActionKind === "album" ? verbs.album(item) : verbs.song(item))
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

    readonly property real headerH: height * 0.10
    /// UI-58: adaptive row height — inside the 18% band it scales, past it a
    /// row is cut. The baseline (0.12 of the column) is captured on the
    /// first real measure.
    property real _rowBase: 0
    onHeightChanged: if (_rowBase <= 0 && height > 0) _rowBase = height * 0.12
    readonly property real rowH: Theme.adaptiveRow(height, 0.12, root._rowBase)
    // Snap the viewport to full rows so the footer never slices one in half.
    readonly property real listH: Math.max(rowH,
        Math.floor((height - headerH) / rowH) * rowH)

    Column {
        anchors.fill: parent
        spacing: 0

        Item {
            id: header
            width: parent.width
            height: root.headerH

            FadeText {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                text: root.title
                color: root.active ? Palette.primary : Palette.onSurfaceVariant
                font.pixelSize: root.headerH * 0.42
                font.bold: root.active
            }

            // VI-4: the selection count sits beside the result count.
            SelectionBadge {
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
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Math.max(1, parent.height * Theme.hairlineRatio)
                color: root.active ? Palette.primary : Palette.outlineVariant
            }
        }

        Item {
            width: parent.width
            height: root.listH

            // UI-48: the shared scroll body carries the overflow stage, so
            // the highlight bleeds past the row (clipped at the gutter edge)
            // while rows still clip vertically at the list bounds.
            ScrollContent {
                id: content
                anchors.fill: parent
                bleed: root.overflow
                model: root.model
                selectable: quickActionKind !== ""
                // ARC-UI-2 M1: the columns paint the same three cells.
                searchFields: ["title", "subtitle", "meta"]

                rowDelegate: Component {
                    Rectangle {
                        id: row
                        required property int index
                        required property string title
                        required property string subtitle
                        required property string meta
                        required property string cover
                        required property bool starred

                        readonly property bool current: index === root.cursor
                        // VI-4: the row sits inside the visual selection range.
                        readonly property bool selectedRow: content.selected(index)
                        // UI-45: the focused cursor or any hovered row leads.
                        readonly property bool emphasized: (row.current && root.active) || rowHover.hovered

                        width: ListView.view.width
                        height: root.rowH
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

                        Rectangle {
                            id: thumb
                            anchors.left: parent.left
                            anchors.verticalCenter: parent.verticalCenter
                            width: row.height * 0.72
                            height: row.height * 0.72
                            radius: root.roundedArt ? width * 0.5 : Theme.radius
                            color: Palette.surfaceContainerHighest
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

                        // Two-line cell: title over a muted subtitle, never one
                        // string in one colour.
                        Column {
                            anchors.left: thumb.right
                            anchors.leftMargin: row.height * 0.18
                            anchors.right: metaLabel.left
                            anchors.rightMargin: row.height * 0.18
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: row.height * 0.04

                            SearchText {
                                id: titleText
                                width: parent.width
                                text: row.title
                                color: (row.current && root.active) || row.selectedRow ? Palette.onSecondaryContainer : Palette.onSurface
                                font.pixelSize: Theme.clamp(row.height * 0.3, 12, 18)
                                elide: Text.ElideRight
                                animated: false
                                query: content.searchQuery
                                currentMatch: row.index === content.searchCurrentRow
                                transformOrigin: Item.Left
                                scale: (row.emphasized && !Theme.reduceMotion)
                                    ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                                    : 1.0

                                Behavior on scale {
                                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                            }

                            Row {
                                width: parent.width
                                spacing: row.height * 0.06
                                visible: row.subtitle.length > 0

                                // UI-56: the artist column carries no meta, so
                                // the heart leads its count subtitle instead.
                                Icon {
                                    id: subtitleHeart
                                    visible: row.starred && row.meta.length === 0
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: subtitleLabel.font.pixelSize * 0.8
                                    height: width
                                    name: "baseline-favorite"
                                    symbolColor: Theme.iconStrong
                                }

                                SearchText {
                                    id: subtitleLabel
                                    width: Math.max(0, parent.width
                                        - (subtitleHeart.visible
                                            ? subtitleHeart.width + parent.spacing : 0))
                                    text: row.subtitle
                                    color: (row.current && root.active) || row.selectedRow ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                                    font.pixelSize: Theme.clamp(row.height * 0.24, 10, 14)
                                    elide: Text.ElideRight
                                    animated: false
                                    query: content.searchQuery
                                    currentMatch: row.index === content.searchCurrentRow
                                }
                            }
                        }

                        // UI-56: the favourite heart leads the meta text (a
                        // state mark, no hitbox); unstarred rows draw none.
                        Icon {
                            visible: row.starred && row.meta.length > 0
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.right: metaLabel.left
                            anchors.rightMargin: row.height * 0.06
                            width: metaLabel.font.pixelSize * 0.8
                            height: width
                            name: "baseline-favorite"
                            symbolColor: Theme.iconStrong
                        }

                        SearchText {
                            id: metaLabel
                            anchors.right: parent.right
                            anchors.rightMargin: row.height * 0.18
                            anchors.verticalCenter: parent.verticalCenter
                            text: row.meta
                            color: (row.current && root.active) || row.selectedRow ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                            font.pixelSize: Theme.clamp(row.height * 0.26, 10, 14)
                            animated: false
                            query: content.searchQuery
                            currentMatch: row.index === content.searchCurrentRow
                        }

                        // Hover only signals clickability (S-5); the cursor moves
                        // on click or keys, never on hover.
                        HoverHandler {
                            id: rowHover
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            onTapped: root.picked(row.index)
                        }
                    }
                }
            }

            FadeText {
                anchors.centerIn: parent
                visible: root.hasQuery && !root.loading && root.model && root.model.count === 0
                text: "no matches"
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.055
            }

            // Skeleton while the request is in flight (shared UI-22 pattern).
            SkeletonList {
                anchors.fill: parent
                visible: root.loading && root.model && root.model.count === 0
            }
        }
    }
}
