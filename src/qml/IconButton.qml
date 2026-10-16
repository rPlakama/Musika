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

// Square icon button. Draws a text glyph, or an SVG when `iconSource` is set.
// Size comes from the layout (or the parent); the glyph is a fraction of it.
//
// SVG style (roadmap D-6), for future icons:
//   - always `VectorImage` + `preferredRendererType: CurveRenderer`; the
//     default GeometryRenderer flattens curves and aliases at icon sizes;
//   - assets live in `src/icons/` (embedded as `qrc:/app/icons/*.svg` by the
//     second `include_bytes_qml!` in `main.rs`) with white fills baked in —
//     VectorImage cannot tint;
//   - each layer draws inside a fixed transparent square, and multi-variant
//     sets share a baseline, so swapping icons never moves anything.
//
// State layers (R6): hover 8%, focus 10% primary + hairline, press 10%,
// disabled 12%/38%. Never colour alone.
Rectangle {
    id: root

    property string symbol: "▶"
    // Optional SVG icon; wins over `symbol` when set.
    property url iconSource: ""
    property real symbolRatio: 0.5
    // Text-glyph colour; play/pause overrides it with the fixed
    // `Theme.iconStrong` (white, UI-31). SVG icons carry their own fill.
    property color symbolColor: Palette.onSurface
    property alias hovered: hover.hovered

    signal clicked()

    /// Fires the "press" feel through the shared `PressBounce` animation: a
    /// quick scale-down, then back with an overshooting bezier. Transport
    /// buttons call this so a toggle/skip is noticeable on keyboard, mouse,
    /// IPC and backend-driven events alike.
    function press() {
        pressFx.press();
    }

    PressBounce {
        id: pressFx
        target: root
    }

    radius: Theme.radius
    color: "transparent"
    border.width: activeFocus ? Math.max(1, width * Theme.hairlineRatio) : 0
    border.color: Palette.primary

    // Press state is animation-driven only; the scale transform lives in
    // `PressBounce` so every button shares one implementation.

    Rectangle {
        anchors.fill: parent
        radius: root.radius
        color: {
            if (!root.enabled)
                return Theme.stateDisabledContainer;
            if (tap.pressed)
                return Theme.statePress;
            if (root.activeFocus)
                return Theme.stateFocus;
            if (hover.hovered)
                return Theme.stateHover;
            return "transparent";
        }

        Behavior on color {
            ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    // Glyph crossfade (UI-31): when the drawn glyph swaps (▶/⏸), the two
    // layers melt into each other over `quick` (120 ms) instead of blinking.
    // Two layers alternate roles; the incoming one fades 0→1 while the
    // outgoing one fades 1→0.
    property string _glyphKey: ""
    property bool _flipGlyphs: false

    function glyphKey() {
        return root.iconSource.toString().length > 0
            ? root.iconSource.toString()
            : root.symbol;
    }

    function applyGlyph(layer) {
        if (root.iconSource.toString().length > 0) {
            layer.iconGlyph = root.iconSource;
            layer.textGlyph = "";
        } else {
            layer.textGlyph = root.symbol;
            layer.iconGlyph = "";
        }
    }

    function advanceGlyph() {
        var key = glyphKey();
        if (key === root._glyphKey)
            return;
        root._glyphKey = key;
        if (root._flipGlyphs) {
            applyGlyph(glyphA);
            glyphA.opacity = 1;
            glyphB.opacity = 0;
        } else {
            applyGlyph(glyphB);
            glyphB.opacity = 1;
            glyphA.opacity = 0;
        }
        root._flipGlyphs = !root._flipGlyphs;
    }

    onSymbolChanged: advanceGlyph()
    onIconSourceChanged: advanceGlyph()

    Component.onCompleted: {
        root._glyphKey = glyphKey();
        applyGlyph(glyphA);
    }

    // Invisible positional frames: each layer is a fixed square that never
    // resizes, and the glyph is centred inside it, so swapping icons (volume
    // tiers, ▶/⏸) only changes the drawing — never the icon's position.
    Rectangle {
        radius: Theme.radius
        id: glyphA
        anchors.centerIn: parent
        width: Math.min(root.width, root.height)
        height: width
        color: "transparent"
        property string textGlyph: ""
        property url iconGlyph: ""
        opacity: 1

        Text {
            anchors.centerIn: parent
            visible: glyphA.iconGlyph.toString().length === 0 && glyphA.textGlyph.length > 0
            text: glyphA.textGlyph
            font.family: Theme.fonts.body
            font.pixelSize: root.width * root.symbolRatio
            color: !root.enabled ? Theme.stateDisabledContent : root.symbolColor
        }

        SymbolIcon {
            anchors.centerIn: parent
            visible: glyphA.iconGlyph.toString().length > 0
            source: glyphA.iconGlyph
            symbolColor: root.enabled ? root.symbolColor : Theme.stateDisabledContent
            width: root.width * root.symbolRatio
            height: width
        }

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    Rectangle {
        radius: Theme.radius
        id: glyphB
        anchors.centerIn: parent
        width: Math.min(root.width, root.height)
        height: width
        color: "transparent"
        property string textGlyph: ""
        property url iconGlyph: ""
        opacity: 0

        Text {
            anchors.centerIn: parent
            visible: glyphB.iconGlyph.toString().length === 0 && glyphB.textGlyph.length > 0
            text: glyphB.textGlyph
            font.family: Theme.fonts.body
            font.pixelSize: root.width * root.symbolRatio
            color: !root.enabled ? Theme.stateDisabledContent : root.symbolColor
        }

        SymbolIcon {
            anchors.centerIn: parent
            visible: glyphB.iconGlyph.toString().length > 0
            source: glyphB.iconGlyph
            symbolColor: root.enabled ? root.symbolColor : Theme.stateDisabledContent
            width: root.width * root.symbolRatio
            height: width
        }

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    // Clickable affordance (reuse for every clickable): the hover handler
    // carries the pointing-hand cursor, the tap handler the action. Pair them
    // like this wherever something is clickable.
    HoverHandler {
        id: hover
        cursorShape: Qt.PointingHandCursor
    }

    TapHandler {
        id: tap
        onTapped: root.clicked()
    }
}
