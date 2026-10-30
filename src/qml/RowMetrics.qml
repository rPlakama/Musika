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

// One shared set of row text metrics per scroll surface. The row body and its
// SearchText cells all use the same three font buckets, so keeping a
// FontMetrics instance per row only duplicated identical measurements.
Item {
    visible: false
    width: 0
    height: 0
    id: root

    property real viewWidth: 0
    property real rowHeight: 0

    readonly property FontMetrics title: titleMetrics
    readonly property FontMetrics subtitle: subtitleMetrics
    readonly property FontMetrics meta: metaMetrics

    /// Width reserved for a `00:00:00` timecode in the meta column.
    readonly property real metaReserveWidth: metaMetrics.advanceWidth("00:00:00")

    FontMetrics {
        id: titleMetrics
        font.family: Theme.fonts.body
        font.pixelSize: Theme.rowFont("title", root.viewWidth, root.rowHeight)
    }

    FontMetrics {
        id: subtitleMetrics
        font.family: Theme.fonts.body
        font.pixelSize: Theme.rowFont("subtitle", root.viewWidth, root.rowHeight)
    }

    FontMetrics {
        id: metaMetrics
        font.family: Theme.fonts.body
        font.pixelSize: Theme.rowFont("meta", root.viewWidth, root.rowHeight)
    }
}
