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

// The one motion primitive: a NumberAnimation carrying the app's default
// duration (`quick`) and the standard Bezier curve, so callers write
//
//     Behavior on opacity { Anim {} }
//     Behavior on scale { Anim { duration: Theme.motion.micro } }
//
// A duration/easing passed at the instance still wins. Animations that need a
// different curve family (Linear, OutCubic, OutBack, snapping reduces) stay
// raw so the easing intent is visible at the call site.
NumberAnimation {
    duration: Theme.motion.quick
    easing.type: Easing.Bezier
    easing.bezierCurve: Theme.ease.standard
}
