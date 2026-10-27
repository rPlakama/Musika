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

// One reusable text primitive for the whole app. Static text uses the native
// font rasterizer (cheapest, crisp at the painted size); opt into the
// distance-field renderer with `scalable` when the text is transformed or
// animated (lyrics scale). Anything can still override `font` / `renderType`.
Text {
    id: root

    property bool scalable: false

    font.family: Theme.fonts.body
    renderType: root.scalable ? Text.QtRendering : Text.NativeRendering
}
