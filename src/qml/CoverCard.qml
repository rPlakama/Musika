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

// The one cover-wall cell (MediaList's grid and ArtistEntry's wall): a cover
// with the corner favourite/rating marks, the download glyph, and the title
// plus one muted line. A grid delegate supplies the model roles by name; the
// host only maps its current/selected state and the two tap actions.
//
// The type/layout ratios stay host-tunable so the two walls keep their
// current metrics instead of one being silently restyled.
Rectangle {
    id: root

    // The model roles (populated by the view's delegate mechanism).
    required property int index
    required property string title
    required property string subtitle
    required property string meta
    required property string cover
    required property string songId
    required property string albumId
    required property bool starred
    /// UI-60: 0..=5 (0 = unrated).
    required property int rating

    property bool current: false
    property bool selected: false
    property bool roundedArt: false
    /// Draw the rating count beside the heart (albums/artists only).
    property bool showRatingMark: false
    /// One muted line: subtitle + meta joined, or the meta alone.
    property bool showSubtitle: true

    property real titleRatio: 0.05
    property real subtitleRatio: 0.038
    property real metaRatio: 0.038
    property real titleTopRatio: 0.02
    property real metaGapRatio: 0.012

    property int firstVisible: -1
    property int lastVisible: -1
    property string searchQuery: ""
    property bool searchCurrent: false

    signal picked(int index)
    signal extendSelection(int index)

    readonly property bool windowed: root.firstVisible >= 0
        && root.lastVisible >= root.firstVisible
        && root.index >= root.firstVisible
        && root.index <= root.lastVisible
    readonly property bool favoriteMark: root.windowed
        && (root.starred || (root.showRatingMark && root.rating > 0))
    readonly property bool cachedMark: root.windowed
        && ((root.songId.length > 0
                && Backend.downloadedIds.indexOf(root.songId) !== -1)
            || (root.albumId.length > 0
                && Backend.cachedAlbumIds.indexOf(root.albumId) !== -1))
    readonly property bool emphasized: (root.current || cellHover.hovered)
        && !Theme.reduceMotion

    width: GridView.view ? GridView.view.cellWidth : 0
    height: GridView.view ? GridView.view.cellHeight : 0
    color: "transparent"

    // The cell block (inset, so neighbouring cards stay separate).
    Rectangle {
        anchors.fill: parent
        anchors.margins: root.width * 0.03
        radius: Theme.radius
        color: (root.current || root.selected)
            ? Palette.secondaryContainer
            : (cellHover.hovered ? Palette.surfaceContainerHigh : "transparent")

        Behavior on color { ColorAnim {} }
    }

    Rectangle {
        id: coverBox
        width: root.width * 0.78
        height: width
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: root.height * 0.05
        radius: root.roundedArt ? width * 0.5 : Theme.radius
        color: Palette.surfaceContainerLowest
        border.color: root.current ? Palette.primary : "transparent"
        border.width: Theme.hairline(root.height)
        clip: true

        transformOrigin: Item.Center
        scale: root.emphasized ? Theme.selectionEmphasis : 1.0

        Behavior on scale { Anim {} }

        CoverImage {
            anchors.fill: parent
            source: root.cover
            // ARC-40: ×1.25 headroom for the UI-50 hover scale.
            sizeFactor: 1.25
        }

        // UI-56/UI-60: the favourite mark rides the cover corner with the
        // rating count at its bottom-right.
        Item {
            visible: root.favoriteMark
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: parent.width * 0.06
            width: parent.width * 0.085
            height: width

            Icon {
                anchors.fill: parent
                visible: root.starred
                name: "baseline-favorite"
                symbolColor: Theme.iconStrong
            }

            RatingBadge {
                visible: root.showRatingMark
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                width: parent.width * 0.84
                height: width
                rating: root.rating
            }
        }

        // The download mark (every track of the song/album cached).
        Icon {
            visible: root.cachedMark
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
        anchors.topMargin: root.height * root.titleTopRatio
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.width * 0.86
        spacing: root.height * root.metaGapRatio

        SearchText {
            id: titleText
            width: parent.width
            text: root.title
            color: Palette.onSurface
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            font.pixelSize: Math.max(10, root.height * root.titleRatio)
            animated: false
            query: root.searchQuery
            currentMatch: root.searchCurrent
            transformOrigin: Item.Center
            scale: root.emphasized
                ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                : 1.0

            Behavior on scale { Anim {} }
        }

        SearchText {
            width: parent.width
            text: root.showSubtitle
                ? Theme.metaJoin([root.subtitle, root.meta])
                : root.meta
            color: Palette.onSurfaceVariant
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            font.pixelSize: Math.max(9, root.height
                * (root.showSubtitle ? root.subtitleRatio : root.metaRatio))
            animated: false
            query: root.searchQuery
            currentMatch: root.searchCurrent
        }
    }

    HoverHandler {
        id: cellHover
        cursorShape: Qt.PointingHandCursor
    }

    TapHandler {
        acceptedModifiers: Qt.ShiftModifier
        onTapped: root.extendSelection(root.index)
    }
    TapHandler {
        acceptedModifiers: Qt.NoModifier
        onTapped: root.picked(root.index)
    }
}
