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
import musika
import "Theme.js" as Theme

// Full-width library list shared by the Albums, Songs and Artists segments:
// header with the Shift-H / Shift-L order cue, then covers, titles and
// right-aligned meta. The wrappers (`AlbumList`, `SongList`, `ArtistList`)
// own the backend channel and map their payload into rows via `setRows`; this
// component owns the rows, the cursor (j/k, gg/G) and activation. Every row is
// a JS object with the normalized fields `title`, `subtitle`, `meta` and
// `cover` (extra fields pass through to the `activated` signal).
Rectangle {
    id: root

    property string title: ""
    property string emptyText: "Nothing indexed yet — run a sync"
    /// UI-22: true while the segment's first payload is in flight.
    property bool loading: false
    // Current order label beside the Shift-H / Shift-L cue; an empty label
    // hides the cue (e.g. Artists, whose order is fixed for now).
    property string orderText: ""
    // Artists wear a circle avatar (the one sanctioned circle, R3).
    property bool roundedArt: false

    signal activated(var item)

    color: Palette.surface

    ListModel {
        id: mediaRows
    }

    function setRows(rows) {
        mediaRows.clear();
        for (var i = 0; i < rows.length; i++)
            mediaRows.append(rows[i]);
        // UI-10: a cursor row exists from the first paint.
        if (mediaRows.count > 0 && (list.currentIndex < 0 || list.currentIndex >= mediaRows.count))
            list.currentIndex = 0;
    }

    function activateCurrent() {
        var item = mediaRows.get(list.currentIndex);
        if (item)
            root.activated(item);
    }

    function handleAction(action) {
        if (mediaRows.count === 0)
            return;
        if (action === "move_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down")
            list.currentIndex = Math.min(mediaRows.count - 1, list.currentIndex + 1);
        else if (action === "top" || action === "bottom")
            Theme.cursorEdge(list, action === "bottom", ListView.Contain);
        else if (action === "enter")
            activateCurrent();
        list.positionViewAtIndex(list.currentIndex, ListView.Contain);
    }

    FadeText {
        anchors.centerIn: parent
        visible: !root.loading && mediaRows.count === 0
        text: root.emptyText
        color: Palette.onSurfaceVariant
        font.pixelSize: Theme.fontSize("body", root.height)
    }

    // UI-22/UI-38: while the first payload is in flight, the shared skeleton
    // over the list area (below the section header) instead of a blank pane.
    SkeletonList {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.topMargin: root.height * 0.1
        visible: root.loading && mediaRows.count === 0
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Section header.
        Item {
            Layout.fillWidth: true
            implicitHeight: root.height * 0.1

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: root.width * 0.02
                text: root.title
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }

            // Order cue: keys -- slash -- current order. Hidden when a segment
            // has no order cycle (Artists).
            Row {
                visible: root.orderText.length > 0
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: root.width * 0.02
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
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: mediaRows
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: row
                required property int index
                required property string title
                required property string subtitle
                required property string meta
                required property string cover

                readonly property real viewWidth: ListView.view ? ListView.view.width : 0
                // Mouse/keyboard parity: the hovered row expands like the cursor.
                readonly property bool emphasized: ListView.isCurrentItem || rowHover.hovered

                width: viewWidth
                height: ListView.view ? ListView.view.height * 0.105 : 0
                color: ListView.isCurrentItem
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

                    FadeText {
                        id: titleText
                        width: parent.width
                        text: row.title
                        color: Palette.onSurface
                        font.pixelSize: row.height * 0.26
                        elide: Text.ElideRight
                        animated: false
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
                    FadeText {
                        width: parent.width
                        text: row.subtitle
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.19
                        elide: Text.ElideRight
                        animated: false
                    }
                }

                FadeText {
                    id: metaLabel
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: row.viewWidth * 0.04
                    text: row.meta
                    color: Palette.onSurfaceVariant
                    font.pixelSize: row.height * 0.19
                    animated: false
                }

                HoverHandler {
                    id: rowHover
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        list.currentIndex = row.index;
                        root.activateCurrent();
                    }
                }
            }
        }
    }
}
