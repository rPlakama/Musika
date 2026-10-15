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
    signal quickAction(var target, var actions)

    QuickActions {
        id: verbs
    }

    /// ARC-I: Alt armed the gesture — anchor on the cursor row.
    function armQuickActions() {
        if (quickActionKind === "" || !model || model.count === 0 || cursor < 0)
            return;
        var item = model.get(cursor);
        var actions = item
            ? (quickActionKind === "artist" ? verbs.artist(item)
                : quickActionKind === "album" ? verbs.album(item) : verbs.song(item))
            : null;
        var row = list.itemAtIndex(cursor);
        if (actions && row)
            quickAction(row, actions);
    }

    // UI-48: the highlight block overflows the row bounds by this much on both
    // sides, filling the column gutters. The overflow stage below clips it at
    // the gutter edge, so it reads as a full-bleed block and never crosses the
    // UI-47 divider.
    readonly property real overflow: width * 0.012

    // UI-41: ease the view to the cursor instead of teleporting.
    ListScroll {
        id: scroll
    }

    readonly property real headerH: height * 0.10
    readonly property real rowH: height * 0.12
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

            // Count badge: contained by its own pill so the number can never
            // read as the next column's title.
            Rectangle {
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

            // UI-48: the highlight clips at the gutter edge, not the list
            // edge. This overflow stage is wider than the column and carries
            // the clip, so the block truly bleeds past the row while
            // scrolling rows still clip vertically at the list bounds.
            Item {
                x: -root.overflow
                width: parent.width + 2 * root.overflow
                height: parent.height
                clip: true

                ListView {
                    id: list
                    anchors.fill: parent
                    anchors.leftMargin: root.overflow
                    anchors.rightMargin: root.overflow
                    clip: false
                    model: root.model
                    currentIndex: root.cursor
                    boundsBehavior: Flickable.StopAtBounds

                    onCurrentIndexChanged: {
                        if (currentIndex >= 0)
                            scroll.ensureVisible(list, currentIndex);
                    }

                    delegate: Rectangle {
                        id: row
                        required property int index
                        required property string title
                        required property string subtitle
                        required property string meta
                        required property string cover

                        readonly property bool current: index === root.cursor
                        // UI-45: the focused cursor or any hovered row leads.
                        readonly property bool emphasized: (row.current && root.active) || rowHover.hovered

                        width: list.width
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

                            FadeText {
                                id: titleText
                                width: parent.width
                                text: row.title
                                color: row.current && root.active ? Palette.onSecondaryContainer : Palette.onSurface
                                font.pixelSize: Theme.clamp(row.height * 0.3, 12, 18)
                                elide: Text.ElideRight
                                animated: false
                                transformOrigin: Item.Left
                                scale: (row.emphasized && !Theme.reduceMotion)
                                    ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                                    : 1.0

                                Behavior on scale {
                                    NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                                }
                            }

                            FadeText {
                                width: parent.width
                                visible: row.subtitle.length > 0
                                text: row.subtitle
                                color: row.current && root.active ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                                font.pixelSize: Theme.clamp(row.height * 0.24, 10, 14)
                                elide: Text.ElideRight
                                animated: false
                            }
                        }

                        FadeText {
                            id: metaLabel
                            anchors.right: parent.right
                            anchors.rightMargin: row.height * 0.18
                            anchors.verticalCenter: parent.verticalCenter
                            text: row.meta
                            color: row.current && root.active ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                            font.pixelSize: Theme.clamp(row.height * 0.26, 10, 14)
                            animated: false
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
