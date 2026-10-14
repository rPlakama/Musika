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
import "Theme.js" as Theme

// UI-41: keyboard scroll easing. Each list owns one instance; `ensureVisible`
// nudges `contentY` with a short animation instead of teleporting, keeping a
// half-row of context around the cursor. Rows that are not realised yet fall
// back to an instant `ListView.Contain`, and `reduceMotion` stays flat.
//
// UI-54: the same instance is the shared scroll-memory seam — `remember`
// stores the cursor + offset (and the focused scope, where one exists) and
// `recall` restores them for a list that was just created or reopened. The
// fullscreen queue, drawer, Main segments and both entry pages all use it.
QtObject {
    id: root

    readonly property NumberAnimation anim: NumberAnimation {
        property: "contentY"
        duration: Theme.motion.quick
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
    }

    function stop() {
        anim.stop();
    }

    function ensureVisible(view, index, pad) {
        if (!view || index < 0)
            return;
        if (Theme.reduceMotion) {
            view.positionViewAtIndex(index, ListView.Contain);
            return;
        }
        var item = view.itemAtIndex(index);
        if (!item) {
            view.positionViewAtIndex(index, ListView.Contain);
            return;
        }
        var margin = pad !== undefined ? pad : item.height * 0.5;
        var top = view.contentY;
        var limit = Math.max(0, view.contentHeight - view.height);
        var target = top;
        if (item.y - margin < top)
            target = item.y - margin;
        else if (item.y + item.height + margin > top + view.height)
            target = item.y + item.height + margin - view.height;
        target = Math.max(0, Math.min(limit, target));
        if (Math.abs(target - top) < 1)
            return;
        anim.stop();
        anim.target = view;
        anim.from = top;
        anim.to = target;
        anim.start();
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
        view.positionViewAtIndex(view.currentIndex, ListView.Contain);
        var y = saved.y;
        Qt.callLater(function () {
            view.contentY = Math.max(0, Math.min(y,
                Math.max(0, view.contentHeight - view.height)));
        });
        return true;
    }
}
