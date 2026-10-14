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

// Reusable press feedback: a quick scale-down and a bouncy return
// (click-click). Attach it to any Item and call `press()`:
//
//     PressBounce { id: fx; target: playButton }
//     ...
//     fx.press()
//
// A transform only, so layout siblings never move. Honors reduceMotion.
// `QtObject` has no default property, so the animation lives in an object
// property instead of being a child.
QtObject {
    id: root

    property Item target: null
    /// How far the item dips before bouncing back.
    property real dip: 0.86

    function press() {
        if (!target || Theme.reduceMotion)
            return;
        root.bounce.restart();
    }

    property SequentialAnimation bounce: SequentialAnimation {
        NumberAnimation {
            target: root.target
            property: "scale"
            to: root.dip
            duration: Theme.motion.micro
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.accelerate
        }
        NumberAnimation {
            target: root.target
            property: "scale"
            to: 1.0
            duration: Theme.motion.quick
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.bounce
        }
    }
}
