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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import QtQuick

// VI-4 + ARC-I-4: the row-state unit extracted from `ScrollContent`
// (CRITIQUESCROLL SC-7) — the visual selection over the cursor range and the
// in-place favourite write. The cursor itself stays the scroll body's (or
// the host's) single source of truth; this object only reads it, so the
// queue-mark work (ARC-UI-4) gets one shared row-update seam.
QtObject {
    id: root

    signal selectionChanged

    /// The rows and the host's cursor.
    property var model: null
    property int cursor: -1
    /// Host gate (e.g. only when the Alt verb kind exists).
    property bool selectable: false

    // Object-valued (a QtObject has no default child list): the anchored
    // range state, shared with every selection surface.
    property Selection sel: Selection {
        onSelectionChanged: root.selectionChanged()
    }

    /// The range depends on the cursor too: move it while active and the
    /// host's badge must follow.
    onCursorChanged: if (root.hasSelection) root.selectionChanged()

    readonly property bool hasSelection: sel.hasSelection
    readonly property int selectionCount: sel.hasSelection
        ? sel.count(root.cursor) : 0

    /// `v`: start a selection on the cursor row, or end the current one.
    function toggle() {
        if (!root.selectable || !root.model || root.model.count === 0
                || root.cursor < 0)
            return;
        if (sel.hasSelection)
            sel.clear();
        else
            sel.start(root.cursor);
    }

    /// `V`: the whole list — the host moves the cursor first, this only
    /// widens the anchor.
    function selectAllRows() {
        if (!root.selectable || !root.model || root.model.count === 0)
            return;
        sel.all();
    }

    /// Shift+motion: keep the anchor, let the host move the cursor to
    /// `index`. Starts a selection on the old cursor when none is live.
    function extendTo(index) {
        if (!root.selectable || !root.model || root.cursor < 0
                || index < 0 || index >= root.model.count)
            return false;
        if (!sel.hasSelection)
            sel.start(root.cursor);
        return true;
    }

    function clear() {
        sel.clear();
    }

    function selected(index) {
        return sel.selected(root.cursor, index);
    }

    /// Ids of the selected rows ([] when there is no selection).
    function ids(field) {
        return sel.ids(root.model, root.cursor, field);
    }

    /// Row indices of the selected range ([] when there is no selection).
    function indices() {
        return sel.indices(root.model, root.cursor);
    }

    /// The selected rows' `starred` flags, aligned with `ids()`.
    function starred(field) {
        return sel.starred(root.model, root.cursor, field);
    }

    /// ARC-I-4: write the favourite state back into the model in place —
    /// the cursor row, or the whole selected range while a selection is
    /// live — so the affected rows' hearts track the Alt verb with no
    /// reload.
    function markStarred(on) {
        if (!root.model || !root.model.setProperty || root.cursor < 0)
            return;
        var first = root.hasSelection
            ? Math.min(sel.anchor, root.cursor) : root.cursor;
        var last = root.hasSelection
            ? Math.max(sel.anchor, root.cursor) : root.cursor;
        for (var i = first; i <= last; ++i)
            root.model.setProperty(i, "starred", on === true);
    }
}
