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

// ARC-UI-2: the shared spawn FX, lifted from the Alt overlay's board. `value`
// rides 0 → 1 on the OutBack spring (one overshoot and settle) and collapses
// back on the plain ease-in; hosts read it for a Scale transform around their
// own pivot (the Alt board's anchor) and keep `visible: value > 0.001` so the
// collapse plays out.
//
// Qt clamps `Easing.Bezier` y > 1, so the CSS spring
// cubic-bezier(.63,.12,.84,1.72) is carried by OutBack instead: the default
// overshoot peaks at ~1.098, right on that curve's crest.
Item {
    id: root

    /// The animated 0..1 value — drive a host transform with it.
    property real value: 0

    NumberAnimation {
        id: spawnIn
        target: root
        property: "value"
        to: 1
        duration: Theme.motion.emphasized
        easing.type: Easing.OutBack
        easing.overshoot: 1.7
    }

    NumberAnimation {
        id: spawnOut
        target: root
        property: "value"
        to: 0
        duration: Theme.motion.quick
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.accelerate
    }

    /// Spring in (`reduceMotion` snaps).
    function play() {
        spawnOut.stop();
        if (Theme.reduceMotion)
            value = 1;
        else
            spawnIn.restart();
    }

    /// Plain ease-in collapse (`reduceMotion` snaps).
    function collapse() {
        spawnIn.stop();
        if (Theme.reduceMotion)
            value = 0;
        else
            spawnOut.restart();
    }
}
