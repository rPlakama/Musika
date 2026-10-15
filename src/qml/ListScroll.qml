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
import "ScrollMemory.js" as Memory

// UI-54: the shared scroll-memory seam — `remember` stores the cursor +
// offset (and the focused scope, where one exists) and `recall` restores
// them for a list that was just created or reopened. The fullscreen queue,
// drawer, Main segments and both entry pages all use it.
//
// `ensureVisible` is the instant jump for semantic teleports only. Cursor
// follow is the view's own highlight machinery (SCROLLCONTENTSTUDY.md), so
// there is no `contentY` animation here to fight teleports or user input.
QtObject {
    id: root

    function ensureVisible(view, index) {
        if (view && index >= 0)
            view.positionViewAtIndex(index, ListView.Contain);
    }

    /// Saves `view`'s cursor and offset under `key`; `scope` is the focused
    /// pane/column that owned the highlight, where the surface has one.
    function remember(key, view, scope) {
        if (key && view)
            Memory.saveScroll(key, view.currentIndex, view.contentY, scope);
    }

    /// Restores a saved cursor + offset into `view` (clamped to its current
    /// content) and guarantees the highlight is visible. Returns false when
    /// there is nothing saved or the view is empty.
    function recall(key, view) {
        var saved = Memory.loadScroll(key);
        if (!saved || !view || view.count === 0)
            return false;
        view.currentIndex = Math.max(0, Math.min(view.count - 1, saved.index));
        // One exact step: assigning `contentY` realises the rows around it.
        // `positionViewAtIndex(Contain)` would paint its own offset first and
        // the saved one would arrive a frame later — the visible flick — and
        // it fights the layout while `contentHeight` is still an estimate
        // (a not-yet-laid-out list has `height == 0` and an inflated estimate,
        // so the clamp below would be wrong).
        var y = saved.y;
        var apply = function () {
            view.contentY = Math.max(0, Math.min(y,
                Math.max(0, view.contentHeight - view.height)));
        };
        apply();
        // Retry once when the list had no real geometry yet; on a laid-out
        // list the first assignment already stuck and no second frame exists.
        if (Math.abs(view.contentY - y) > 1)
            Qt.callLater(apply);
        return true;
    }
}
