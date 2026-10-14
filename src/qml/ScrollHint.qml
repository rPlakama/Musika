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

// UI-13: a slim scroll-position hint for long lists. Instantiate it as a
// direct child of the view it tracks (ListView/Flickable); it anchors itself
// to the view's left edge and draws only — no layout, hover or click
// behaviour. Hidden while the content fits; `reduceMotion` shows/hides it
// without fading.
Item {
    id: root

    property var view: null

    readonly property bool scrollable: view !== null && view.visibleArea.heightRatio < 1
    readonly property bool emphasized: hover.hovered || (view !== null && (view.moving || view.flicking))
    readonly property real thumbHeight: scrollable
        ? Math.max(root.height * view.visibleArea.heightRatio, root.height * 0.02)
        : 0

    anchors.top: parent.top
    anchors.bottom: parent.bottom
    anchors.left: parent.left
    width: view ? Math.max(1, view.height * Theme.hairlineRatio * 0.5) : 0
    visible: opacity > 0.01
    opacity: scrollable ? 1 : 0
    z: 1

    Behavior on opacity {
        NumberAnimation {
            duration: Theme.reduceMotion ? 0 : Theme.motion.quick
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.standard
        }
    }

    Rectangle {
        width: parent.width
        height: root.thumbHeight
        y: root.scrollable ? root.view.visibleArea.yPosition * root.height : 0
        radius: width * 0.5
        color: root.emphasized ? Palette.iconStrong : Palette.onSurface

        Behavior on color {
            ColorAnimation {
                duration: Theme.reduceMotion ? 0 : Theme.motion.quick
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }
    }

    HoverHandler {
        id: hover
    }
}
