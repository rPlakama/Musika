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
import QtQuick.Effects
import "Theme.js" as Theme

// Cover/artist art with a fade-in once decoded (POLISHAUDIT §3). Drop-in for
// the inline `Image { visible: status === Image.Ready }` blocks: the caller
// owns the rounded/clipped container, this owns the pixels and the fade.
// `ambient` is the shared D-5 backdrop mode: the GPU layer blurs and
// desaturates the art, and the caller lays a theme gradient on top.
Image {
    id: root

    property bool ambient: false

    asynchronous: true
    cache: false
    fillMode: Image.PreserveAspectCrop
    opacity: status === Image.Ready ? 1 : 0
    visible: opacity > 0

    layer.enabled: visible && root.ambient
    layer.effect: MultiEffect {
        blurEnabled: true
        blur: 1.0
        blurMax: 64
        saturation: 0.6
        autoPaddingEnabled: false
    }

    Behavior on opacity {
        NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
    }
}
