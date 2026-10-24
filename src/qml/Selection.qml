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

// VI-4: anchored visual row selection for a cursor list. The host owns the
// cursor; this object owns the anchor and derives the selected range from it
// (`selected`/`count`/`ids`), so every surface shares one semantic.
QtObject {
    id: root

    signal selectionChanged

    property bool active: false
    property int anchor: -1

    readonly property bool hasSelection: root.active && root.anchor >= 0

    /// Rows between the anchor and `cursor`, inclusive.
    function count(cursor) {
        return root.hasSelection ? Math.abs(cursor - root.anchor) + 1 : 0;
    }

    function selected(cursor, index) {
        return root.hasSelection
            && index >= Math.min(root.anchor, cursor)
            && index <= Math.max(root.anchor, cursor);
    }

    function start(cursor) {
        if (cursor < 0)
            return;
        root.active = true;
        root.anchor = cursor;
        root.selectionChanged();
    }

    /// Whole list; the host moves its cursor to the last row.
    function all() {
        root.active = true;
        root.anchor = 0;
        root.selectionChanged();
    }

    function clear() {
        if (!root.active && root.anchor < 0)
            return;
        root.active = false;
        root.anchor = -1;
        root.selectionChanged();
    }

    /// `field` ids of the selected rows, in list order.
    function ids(model, cursor, field) {
        var out = [];
        if (!root.hasSelection || !model)
            return out;
        var first = Math.min(root.anchor, cursor);
        var last = Math.max(root.anchor, cursor);
        for (var i = first; i <= last; i++) {
            var item = model.get(i);
            if (item && item[field])
                out.push(item[field]);
        }
        return out;
    }

    /// Row indices of the selected range, in list order ([] without a
    /// selection).
    function indices(model, cursor) {
        var out = [];
        if (!root.hasSelection || !model)
            return out;
        var first = Math.min(root.anchor, cursor);
        var last = Math.max(root.anchor, cursor);
        for (var i = first; i <= last; i++)
            out.push(i);
        return out;
    }

    /// `starred` flags of the selected rows, aligned with `ids()` (same field
    /// filter, same order) — the Alt batch favourite reads the selection's
    /// state from this.
    function starred(model, cursor, field) {
        var out = [];
        if (!root.hasSelection || !model)
            return out;
        var first = Math.min(root.anchor, cursor);
        var last = Math.max(root.anchor, cursor);
        for (var i = first; i <= last; i++) {
            var item = model.get(i);
            if (item && item[field])
                out.push(item.starred === true);
        }
        return out;
    }
}
