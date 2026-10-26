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
// The row is one horizontal strip: cover, metadata, a squeezeable gap, the
// symbol block and the additional info. Metadata is the only flexible text
// segment and wraps when the gap runs out; the symbols always keep their
// rectangle and the additional info keeps its reserved width.
Item {
    id: root

    required property real viewWidth
    required property real rowHeight
    property bool emphasized: false
    property bool current: false
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
    /// The host viewport's rendered band (marks exist only inside it).
    property int visibleStart: -1
    property int visibleEnd: -1
    property int markIndex: 0
    property string markSongId: ""
    property string markAlbumId: ""
    property bool markStarred: false
    property int markRating: 0
    property bool markRatingVisible: false

    property string searchQuery: ""
    property bool searchCurrent: false

    /// The desired breathing room between metadata and symbols. It is spent
    /// first; only when it reaches zero does the metadata segment start
    /// wrapping.
    readonly property real desiredGap: root.viewWidth * 0.03
    readonly property real metaReserve: root.showMeta
        ? metaBox.width + root.rightMargin : 0
    readonly property real marksReserve: root.showMarks && marks.width > 0
        ? marks.width + (root.showMeta ? root.marksMargin : root.rightMargin) : 0
    readonly property real metadataLeft: root.showCover
        ? cover.width + root.textGap : root.leftMargin
    readonly property real metadataAvailable: Math.max(1,
        root.viewWidth - root.metadataLeft - root.metaReserve - root.marksReserve)
    readonly property real metadataMin: Math.max(1, root.viewWidth * 0.18)
    readonly property real gapWidth: Math.max(0,
        Math.min(root.desiredGap, root.metadataAvailable - root.metadataMin))

    Item {
        anchors.fill: parent

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
                sizeFactor: 1.25
            }
        }

        // Metadata segment: the only elastic piece.
        Rectangle {
            id: metainfoBox
            color: "transparent"
            clip: true
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: root.showCover ? cover.right : parent.left
            anchors.leftMargin: root.showCover ? root.textGap : root.leftMargin
            anchors.right: gap.left
            height: root.rowHeight

            Column {
                id: metainfoColumn
                width: parent.width
                anchors.verticalCenter: parent.verticalCenter
                spacing: root.rowHeight * 0.05

                SearchText {
                    id: titleText
                    width: parent.width
                    height: root.showSubtitle ? root.rowHeight * 0.55 : root.rowHeight
                    text: root.title
                    color: root.titleColor
                    font.pixelSize: root.titlePixelSize
                    font.bold: root.current
                    fontSizeMode: Text.Fit
                    minimumPixelSize: 7
                    wrapMode: Text.WrapAnywhere
                    maximumLineCount: 2
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
                    height: root.showSubtitle ? root.rowHeight * 0.36 : 0
                    text: root.subtitle
                    color: root.subtitleColor
                    font.pixelSize: root.subtitlePixelSize
                    fontSizeMode: Text.Fit
                    minimumPixelSize: 7
                    wrapMode: Text.WrapAnywhere
                    maximumLineCount: 2
                    elide: Text.ElideRight
                    animated: false
                    query: root.searchQuery
                    currentMatch: root.searchCurrent
                }
            }
        }

        // Squeezeable gap: its width goes to zero before the metadata wraps.
        Rectangle {
            id: gap
            color: "transparent"
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: root.showMarks && marks.width > 0
                ? marks.left : (root.showMeta ? metaBox.left : parent.right)
            width: root.gapWidth
            height: 1
        }

        // Symbols segment: its own rectangle, always kept before meta.
        RowMarks {
            id: marks
            visible: root.showMarks
            anchors.verticalCenter: parent.verticalCenter
            anchors.right: root.showMeta ? metaBox.left : parent.right
            anchors.rightMargin: root.showMeta ? root.marksMargin : root.rightMargin
            cursor: root.markCursor
            firstVisible: root.visibleStart
            lastVisible: root.visibleEnd
            index: root.markIndex
            songId: root.markSongId
            albumId: root.markAlbumId
            starred: root.markStarred
            rating: root.markRating
            showRating: root.markRatingVisible
            base: metaLabel.font.pixelSize * 0.8
        }

        // Additional info segment: its own reserved rectangle.
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
    }
}
