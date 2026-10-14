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

// ARC-38: icon facade over `SymbolIcon` — one component resolves a short qrc
// name, keeps the palette role explicit, adds an optional status badge and a
// text fallback when the vector cannot be parsed. The fixed frame and
// `iconRatio` semantics are unchanged from `SymbolIcon`.
//
//     Icon { name: "reload-ui-2-svgrepo-com"; symbolColor: Theme.iconStrong }
//     Icon { name: "album"; badge: "↓"; fallbackText: "A" }
//
// Badges stay QML-side (colours + glyph here); they are never encoded in asset
// or desktop-entry names. The badge is `Loader`-gated, so an absent badge
// costs no items.
Item {
    id: root

    /// Short asset name (`"album"` → `qrc:/app/icons/album.svg`); a URL or
    /// absolute path is used verbatim.
    property string name: ""
    /// Explicit source; wins over `name` when set.
    property url source: ""
    /// Glyph colour — pass a palette role.
    property color symbolColor: Palette.onSurface
    property real iconRatio: 1.0
    /// Optional corner badge glyph (e.g. a status mark).
    property string badge: ""
    property color badgeGlyphColor: Palette.onPrimary
    property color badgeBackground: Palette.primary
    /// Shown centred when the source cannot be parsed (empty = blank frame).
    property string fallbackText: ""

    readonly property url resolvedSource: {
        if (root.source.toString().length > 0)
            return root.source;
        if (root.name.length === 0)
            return "";
        if (root.name.indexOf(":") >= 0 || root.name.charAt(0) === "/")
            return root.name;
        return "qrc:/app/icons/" + root.name + ".svg";
    }

    SymbolIcon {
        id: symbol
        anchors.fill: parent
        source: root.resolvedSource
        symbolColor: root.symbolColor
        iconRatio: root.iconRatio
    }

    // A vector parse failure must paint something, never a blank frame.
    FadeText {
        anchors.centerIn: parent
        visible: !symbol.ready
        text: root.fallbackText
        color: root.symbolColor
        font.pixelSize: Math.min(root.width, root.height) * 0.6
        animated: false
    }

    Loader {
        active: root.badge.length > 0
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        width: Math.min(root.width, root.height) * 0.42
        height: width

        sourceComponent: Rectangle {
            radius: width * 0.5
            color: root.badgeBackground
            border.color: Palette.surface
            border.width: Math.max(1, width * Theme.hairlineRatio)

            FadeText {
                anchors.centerIn: parent
                text: root.badge
                color: root.badgeGlyphColor
                font.pixelSize: parent.height * 0.62
                animated: false
            }
        }
    }
}
