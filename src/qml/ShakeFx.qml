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
import "Theme.js" as Theme

// UI-49: reusable "denied input" feedback — a short horizontal shake plus a
// small scale bounce on the same target. The offset goes into a Translate so
// nothing shifts layout:
//
//     ShakeFx { id: fx; target: field }
//     TextField { id: field; transform: Translate { x: fx.offset } }
//     ...
//     fx.shake()
//
// Keyframes total 5 x `flash` (150 ms), flat under reduceMotion. `QtObject`
// has no default property, so the animation lives in an object property.
QtObject {
    id: root

    property Item target: null
    /// Horizontal offset for the caller's Translate (a transform, never `x`).
    property real offset: 0

    function shake() {
        if (!target || Theme.reduceMotion)
            return;
        root.swing.restart();
    }

    property ParallelAnimation swing: ParallelAnimation {
        SequentialAnimation {
            NumberAnimation { target: root; property: "offset"; to: 4.2; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            NumberAnimation { target: root; property: "offset"; to: -3.5; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            NumberAnimation { target: root; property: "offset"; to: 2.8; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            NumberAnimation { target: root; property: "offset"; to: -1.4; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            NumberAnimation { target: root; property: "offset"; to: 0; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
        SequentialAnimation {
            NumberAnimation { target: root.target; property: "scale"; to: 0.97; duration: Theme.motion.flash; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.accelerate }
            NumberAnimation { target: root.target; property: "scale"; to: 1.0; duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.bounce }
        }
    }
}
