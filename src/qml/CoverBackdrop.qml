// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

import QtQuick
import musika

// Album/artist/fullscreen ambience: soft radial pools behind the sharp
// artwork. `showCover` lets the fullscreen split own its own cover pane.
Item {
    id: root

    property url source
    property bool showCover: true
    /// True once the sharp artwork is decoded.
    readonly property bool ready: cover.ready

    /// Optional ceiling for hosts that reserve a rows band below the art.
    property real maxSide: Number.POSITIVE_INFINITY

    /// The largest square that fits the page; the cover grows/shrinks with it.
    readonly property real coverSide: Math.min(width, height, maxSide)

    clip: true

    Rectangle {
        anchors.fill: parent
        color: "#000000"
    }

    // The artwork itself: top-centre, aspect-fit in the largest page square.
    CoverImage {
        id: cover
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        width: root.coverSide
        height: root.coverSide
        source: root.showCover ? root.source : ""
        fillMode: Image.PreserveAspectFit
        visible: root.showCover
        opacity: visible && ready ? 1 : 0
    }
}
