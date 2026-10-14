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

// UI-41: keyboard scroll easing. Each list owns one instance; `ensureVisible`
// nudges `contentY` with a short animation instead of teleporting, keeping a
// half-row of context around the cursor. Rows that are not realised yet fall
// back to an instant `ListView.Contain`, and `reduceMotion` stays flat.
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
}
