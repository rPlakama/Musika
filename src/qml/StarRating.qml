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

// UI-60: the five-star rating array, shared by the album covers and the
// ArtistEntry strip. Hover previews the level incrementally up to the
// pointer; clicking a level sets it, clicking the lit top clears it to 0.
// The host owns the backend call and optimistic state.
Row {
    id: root

    /// 0..=5 (0 = unrated; all five outlines).
    property int rating: 0
    /// The hitbox side of one star; the glyph is 64% of it.
    property real unit: 24
    signal picked(int rating)


    property int _hovered: -1
    // UI-60: the freshly added star scales up once (the row-selection
    // emphasis), then settles.
    property int lastRating: 0
    property int pulseIndex: -1

    Component.onCompleted: root.lastRating = root.rating
    onRatingChanged: {
        if (root.rating > root.lastRating)
            root.pulseIndex = root.rating - 1;
        root.lastRating = root.rating;
    }

    Timer {
        id: pulseTimer
        interval: Theme.motion.quick
        onTriggered: root.pulseIndex = -1
    }
    onPulseIndexChanged: if (root.pulseIndex >= 0) pulseTimer.restart()

    spacing: root.unit * 0.05

    Repeater {
        model: 5

        delegate: Item {
            id: starSlot

            required property int index

            readonly property bool lit: index < root.rating
                || index <= root._hovered

            width: root.unit
            height: width
            transformOrigin: Item.Center
            scale: starSlot.index === root.pulseIndex ? Theme.selectionEmphasis : 1.0

            Behavior on scale { Anim {} }

            SymbolIcon {
                id: starGlyph
                anchors.centerIn: parent
                width: root.unit * 0.64
                height: width
                source: starSlot.lit
                    ? "qrc:/app/icons/star.svg"
                    : "qrc:/app/icons/star-outline.svg"
                symbolColor: starSlot.lit ? Palette.primary : Palette.onSurfaceVariant
            }

            PressBounce {
                id: starPress
                target: starGlyph
            }

            HoverHandler {
                id: starHover
                cursorShape: Qt.PointingHandCursor
                onHoveredChanged: {
                    if (starHover.hovered) {
                        root._hovered = starSlot.index;
                        starPress.press();
                    } else if (root._hovered === starSlot.index) {
                        root._hovered = -1;
                    }
                }
            }

            TapHandler {
                onTapped: {
                    var level = starSlot.index + 1;
                    root.picked(root.rating === level ? 0 : level);
                }
            }
        }
    }
}
