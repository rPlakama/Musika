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

// UI-60: the one favourite button, with the rating count built in. Two
// layouts keep the same state story everywhere:
//
//   variant: "chip"  — the star sits centred in an ActionChip and the count
//                      is the corner badge (fullscreen keybar, ArtistEntry).
//   variant: "row"   — the heart and the count are two symbols in one Row
//                      (the shared list/album marks).
//
// Hosts own interaction (an ActionChip or the Alt overlay); this component
// owns the glyph choice and the 0..=5 count so no surface re-implements it.
Item {
    id: root

    property bool starred: false
    property int rating: 0
    property bool showGlyph: true
    property bool showCount: true
    /// Glyph box: the star box in chip mode, the heart block in row mode.
    property real unit: 24
    property string variant: "chip"

    property color starredColor: Theme.favoriteStar
    property color unstarredColor: Palette.onSurfaceVariant
    property color countColor: Palette.primary
    property color countBackground: Palette.surface

    implicitWidth: root.variant === "row" ? rowSymbols.implicitWidth : root.unit
    implicitHeight: root.variant === "row" ? rowSymbols.implicitHeight : root.unit

    // Chip form: a star (outline while unfavourited) with the count badge at
    // the chip's bottom-right corner.
    Icon {
        visible: root.variant === "chip" && root.showGlyph
        anchors.centerIn: parent
        width: root.unit
        height: width
        source: root.starred
            ? "qrc:/app/icons/star.svg"
            : "qrc:/app/icons/star-outline.svg"
        symbolColor: root.starred ? root.starredColor : root.unstarredColor
    }

    RatingBadge {
        visible: root.variant === "chip" && root.showCount
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: parent.width * 0.04
        width: parent.width * 0.588
        height: width
        rating: root.rating
        symbolColor: root.countColor
        backgroundColor: root.countBackground
    }

    // Row form: heart + count as two symbols sharing one centre axis.
    Row {
        id: rowSymbols
        visible: root.variant === "row"
        anchors.centerIn: parent
        spacing: root.unit * 0.208

        Rectangle {
            visible: root.showGlyph
            anchors.verticalCenter: parent.verticalCenter
            width: root.unit
            height: width
            color: "transparent"

            Icon {
                anchors.centerIn: parent
                width: root.unit * 0.833
                height: width
                source: "qrc:/app/icons/baseline-favorite.svg"
                symbolColor: Theme.iconStrong
            }
        }

        RatingBadge {
            visible: root.showCount
            anchors.verticalCenter: parent.verticalCenter
            width: root.unit * 1.575
            height: width
            glyphInset: 0.06
            rating: root.rating
            symbolColor: root.countColor
            backgroundColor: root.countBackground
        }
    }
}
