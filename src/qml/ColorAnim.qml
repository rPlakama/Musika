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

// The colour twin of `Anim`: a ColorAnimation with the default `quick`
// duration and the standard Bezier curve.
//
//     Behavior on color { ColorAnim {} }
//     Behavior on border.color { ColorAnim { duration: Theme.motion.micro } }
ColorAnimation {
    duration: Theme.motion.quick
    easing.type: Easing.Bezier
    easing.bezierCurve: Theme.ease.standard
}
