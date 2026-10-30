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
    // A page pushed during session bootstrap exists before the first layout
    // (width/height 0). Do not decode a 32 px cover there and then reload it
    // at the real size; wait for the first real geometry and load once.
    readonly property bool sized: width > 0 && height > 0
    // Session bootstrap creates the page before the shell settles; the cover
    // box can pass through 0 -> odd interim sizes -> final. Keep the image
    // unloaded until the geometry has been still for a moment, then load
    // once at the settled size. Once painted, geometry changes must not
    // clear the source.
    property bool _geometryReady: false
    property bool _hasLoaded: false
    readonly property bool canLoad: root.sized
        && (root._geometryReady || root._hasLoaded)
    Timer {
        id: settleTimer
        interval: 32
        repeat: false
        onTriggered: root._geometryReady = true
    }
    Component.onCompleted: settleTimer.start()
    onWidthChanged: {
        if (!root._hasLoaded)
            settleTimer.restart();
    }
    onHeightChanged: {
        if (!root._hasLoaded)
            settleTimer.restart();
    }
    readonly property int decodeWidth: Math.min(root.maxDecodeEdge, Math.max(32,
        Math.ceil(width * root.decodeFactor / 32) * 32))
    readonly property int decodeHeight: Math.min(root.maxDecodeEdge, Math.max(32,
        Math.ceil(height * root.decodeFactor / 32) * 32))

    // The thumb path is one pre-scaled file per cover: decode it once at its
    // native edge so every row using the same cover hits the same cached
    // pixmap instead of a different per-row sourceSize.
    readonly property bool usingThumb: root.imageSource.toString().indexOf(".thumb.jpg") >= 0
    readonly property bool ready: image.status === Image.Ready
    opacity: root.ready ? 1 : 0
    visible: root.ready

    // A thumbnail the cache has not written yet falls back to the full cover.
    property bool thumbFailed: false
    onSourceChanged: root.thumbFailed = false

    // Keep in sync with `THUMB_EDGE` in src/covers.rs.
    readonly property int thumbEdge: 320

    // A display-sized copy (`raw-hash_768x768.jpg`) already is the DPI
    // target. Deriving `…_768x768.thumb.jpg` from it would point at a file
    // the cache never writes; keep the display copy as-is when the surface
    // shrinks below the thumb edge (the sourceSize still downscales it).
    readonly property var _displayMatch: /_([0-9]+)x[0-9]+\./.exec(root.source.toString())
    readonly property int displayEdge: root._displayMatch ? Number(root._displayMatch[1]) : 0
    readonly property bool displayCopy: root.displayEdge > 0

    // Decode a display copy at its own bucket, not at the live painted size:
    // resizing inside one bucket must not restart the async decode every
    // 32 px (the cover would blink while the window moves).
    readonly property int sourceWidth: root.usingThumb
        ? root.thumbEdge : (root.displayCopy ? root.displayEdge : root.decodeWidth)
    readonly property int sourceHeight: root.usingThumb
        ? root.thumbEdge : (root.displayCopy ? root.displayEdge : root.decodeHeight)

    function thumbUrl(u) {
        var text = u.toString()
        var slash = text.lastIndexOf("/")
        var dot = text.lastIndexOf(".")
        return (dot <= slash ? text : text.substring(0, dot)) + ".thumb.jpg"
    }

    readonly property url imageSource: {
        var text = root.source ? root.source.toString() : ""
        if (text.length === 0 || text === "undefined")
            return ""
        if (text.indexOf("file:") !== 0)
            return root.source
        if (root.thumbFailed || root.displayCopy)
            return root.source
        var deviceEdge = Math.max(root.decodeWidth, root.decodeHeight) * Screen.devicePixelRatio
        return deviceEdge <= root.thumbEdge ? root.thumbUrl(text) : root.source
    }

    Image {
        id: image
        anchors.fill: parent
        source: root.canLoad ? root.imageSource : ""
        sourceSize.width: root.sourceWidth
        sourceSize.height: root.sourceHeight
        asynchronous: true
        // MEMORY: Qt's image cache keeps decoded covers after their delegate
        // dies, so scrolling a wall or changing tabs grows without bound.
        // Decode only for the delegates that are alive; the thumbnails keep
        // re-decodes cheap.
        cache: false
        fillMode: root.fillMode
        // Cover art is scaled in the page headers and rows: keep bilinear
        // filtering and mipmaps enabled for a clean downscale.
        smooth: true
        mipmap: true
        visible: root.ready
        onStatusChanged: {
            if (status === Image.Ready)
                root._hasLoaded = true;
            if (status === Image.Error && !root.thumbFailed && !root.displayCopy
                    && root.imageSource.toString() !== root.source.toString())
                root.thumbFailed = true
        }
    }

    Behavior on opacity { Anim {} }
}
