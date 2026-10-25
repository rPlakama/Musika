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
import QtQuick.Window
import "Theme.js" as Theme

// Cover/artist art with a fade-in once decoded (POLISHAUDIT §3). Drop-in for
// the inline `Image { visible: status === Image.Ready }` blocks: the caller
// owns the rounded/clipped container, this owns the pixels and the fade.
// ARC-40: the decode is bounded per surface — `sizeFactor` adds headroom over
// the painted size (list/grid thumbs 1.25 for the UI-50 hover scale, hero
// covers 1), and everything is rounded to a 32 px grid so resizes do not
// reload the image per pixel. `sourceSize` is in device-independent pixels; Qt multiplies it by the
// window DPR before calling QImageReader::setScaledSize (which preserves the
// aspect ratio). Small surfaces load the pre-scaled thumbnail Rust writes into
// the cover cache, so the single Qt decode thread only touches a 320 px file;
// large surfaces decode the provider's original art at their painted size.
Item {
    id: root

    /// Cover URL (`file://…`) as resolved by the Rust cover cache.
    property url source

    /// Decode headroom over the painted size (1.0 = exactly the painted size).
    property real sizeFactor: 1.0

    /// How the decoded pixels fill this item. The default keeps the historic
    /// cover-thumbnail crop; backdrops ask for `PreserveAspectFit` so the
    /// album square is never cut.
    property int fillMode: Image.PreserveAspectCrop

    // The cache keeps the provider's original art, so large surfaces can
    // decode at their painted size. Cap pathological uploads to a sane edge.
    readonly property int maxDecodeEdge: 4096
    readonly property real decodeFactor: root.sizeFactor
    readonly property int decodeWidth: Math.min(root.maxDecodeEdge, Math.max(32,
        Math.ceil(width * root.decodeFactor / 32) * 32))
    readonly property int decodeHeight: Math.min(root.maxDecodeEdge, Math.max(32,
        Math.ceil(height * root.decodeFactor / 32) * 32))

    readonly property bool ready: image.status === Image.Ready
    opacity: root.ready ? 1 : 0
    visible: root.ready

    // A thumbnail the cache has not written yet falls back to the full cover.
    property bool thumbFailed: false
    onSourceChanged: root.thumbFailed = false

    // Keep in sync with `THUMB_EDGE` in src/covers.rs.
    readonly property int thumbEdge: 320

    function thumbUrl(u) {
        var text = u.toString()
        var slash = text.lastIndexOf("/")
        var dot = text.lastIndexOf(".")
        return (dot <= slash ? text : text.substring(0, dot)) + ".thumb.jpg"
    }

    readonly property url imageSource: {
        var text = root.source.toString()
        if (text.length === 0)
            return ""
        if (text.indexOf("file:") !== 0)
            return root.source
        if (root.thumbFailed)
            return root.source
        var deviceEdge = Math.max(root.decodeWidth, root.decodeHeight) * Screen.devicePixelRatio
        return deviceEdge <= root.thumbEdge ? root.thumbUrl(text) : root.source
    }

    Image {
        id: image
        anchors.fill: parent
        source: root.imageSource
        sourceSize.width: root.decodeWidth
        sourceSize.height: root.decodeHeight
        asynchronous: true
        cache: true
        fillMode: root.fillMode
        visible: root.ready
        onStatusChanged: {
            if (status === Image.Error && !root.thumbFailed
                    && root.imageSource.toString() !== root.source.toString())
                root.thumbFailed = true
        }
    }

    Behavior on opacity {
        NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
    }
}
