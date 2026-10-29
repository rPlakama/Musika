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

// The one modal scrim: a full-bleed dim layer that eases in/out with the
// surface it protects, and optionally absorbs a click to dismiss.
//
//     Scrim { shown: root.opened; tapToDismiss: true; onDismissed: root.close() }
//
// Hosts keep their `z`; the component owns the fill, opacity and click layer.
// `visible` follows the fade so a closed scrim costs no paint, while the
// opacity Behavior still plays the close.
Rectangle {
    id: root

    property bool shown: false
    /// Target opacity while shown (`Theme.scrimOpacity` is the modal default).
    property real strength: Theme.scrimOpacity
    property color tint: Palette.scrim
    property int duration: Theme.motion.standard
    property var easingCurve: Theme.ease.standard
    /// Clicking the scrim dismisses the surface (search, queue, lyrics).
    property bool tapToDismiss: false

    signal dismissed()

    anchors.fill: parent
    color: root.tint
    opacity: root.shown ? root.strength : 0
    visible: opacity > 0

    Behavior on opacity {
        Anim {
            duration: root.duration
            easing.bezierCurve: root.easingCurve
        }
    }

    TapHandler {
        enabled: root.tapToDismiss && root.shown
        onTapped: root.dismissed()
    }
}
