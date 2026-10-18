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

// ARC-IDK: one row body shared by library lists and the fullscreen queue.
// Covers, text, meta and the shared RowMarks keep their own update seams
// (cover source, title/subtitle text, meta text, mark ids), so a per-row
// change never has to recall the surrounding list.
Item {
    id: root

    required property real viewWidth
    required property real rowHeight
    property bool emphasized: false
    property bool roundedArt: false

    property bool showCover: true
    property bool showSubtitle: true
    property bool showMeta: true
    property bool showMarks: true

    property string cover: ""
    property string title: ""
    property string subtitle: ""
    property string meta: ""

    property real titlePixelSize: rowHeight * 0.26
    property real subtitlePixelSize: rowHeight * 0.19
    property real metaPixelSize: rowHeight * 0.19
    property color titleColor: Palette.onSurface
    property color subtitleColor: Palette.onSurfaceVariant
    property color metaColor: Palette.onSurfaceVariant

    property real leftMargin: viewWidth * 0.04
    property real rightMargin: viewWidth * 0.04
    property real textGap: viewWidth * 0.04
    property real marksMargin: viewWidth * 0.01275
    property bool reserveMeta: false
    property bool metaAlignRight: false

    property int markCursor: -1
    property int markIndex: 0
    property string markSongId: ""
    property string markAlbumId: ""
    property bool markStarred: false
    property int markRating: 0
    property bool markRatingVisible: false

    property string searchQuery: ""
    property bool searchCurrent: false

    Item {
        width: root.width
        height: root.height

            Rectangle {
                id: cover
        visible: root.showCover
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: root.leftMargin
        width: root.rowHeight * 0.78
        height: root.rowHeight * 0.78
        radius: root.roundedArt ? width * 0.5 : Theme.radius
        color: Palette.surfaceContainerHigh
        clip: true
        scale: root.emphasized && !Theme.reduceMotion ? Theme.selectionEmphasis : 1.0

        Behavior on scale {
            NumberAnimation {
                duration: Theme.motion.quick
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }

        CoverImage {
            anchors.fill: parent
            source: root.cover
            sizeFactor: 2
        }
    }

    Column {
        id: textColumn
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: root.showCover ? cover.right : parent.left
        anchors.leftMargin: root.showCover ? root.textGap : root.leftMargin
        anchors.right: root.showMeta ? metaBox.left : parent.right
        anchors.rightMargin: root.showMeta ? root.textGap : root.marksMargin
        spacing: root.rowHeight * 0.05

        SearchText {
            id: titleText
            width: parent.width
            text: root.title
            color: root.titleColor
            font.pixelSize: root.titlePixelSize
            elide: Text.ElideRight
            animated: false
            query: root.searchQuery
            currentMatch: root.searchCurrent
            transformOrigin: Item.Left
            scale: root.emphasized && !Theme.reduceMotion
                ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                : 1.0

            Behavior on scale {
                NumberAnimation {
                    duration: Theme.motion.quick
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }
        }

        SearchText {
            visible: root.showSubtitle
            width: parent.width
            text: root.subtitle
            color: root.subtitleColor
            font.pixelSize: root.subtitlePixelSize
            elide: Text.ElideRight
            animated: false
            query: root.searchQuery
            currentMatch: root.searchCurrent
        }
    }

    Rectangle {
        id: metaBox
        visible: root.showMeta
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: root.rightMargin
        width: root.reserveMeta
            ? metaMetrics.advanceWidth("00:00:00")
            : metaLabel.implicitWidth
        height: metaLabel.implicitHeight
        color: "transparent"

        FontMetrics {
            id: metaMetrics
            font.family: Theme.fonts.body
            font.pixelSize: metaLabel.font.pixelSize
        }

        SearchText {
            id: metaLabel
            anchors.fill: parent
            text: root.meta
            color: root.metaColor
            font.pixelSize: root.metaPixelSize
            horizontalAlignment: root.metaAlignRight ? Text.AlignRight : Text.AlignLeft
            animated: false
            query: root.searchQuery
            currentMatch: root.searchCurrent
        }
    }

    RowMarks {
        visible: root.showMarks
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: root.showMeta ? metaBox.left : parent.right
        anchors.rightMargin: root.showMeta ? root.marksMargin : root.rightMargin
        cursor: root.markCursor
        index: root.markIndex
        songId: root.markSongId
        albumId: root.markAlbumId
        starred: root.markStarred
        rating: root.markRating
        showRating: root.markRatingVisible
        base: metaLabel.font.pixelSize * 0.8
        }
    }
}
