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

// ARC-UI-2 M1: the viewport-scoped row snapshot extracted from
// `ScrollContent` (CRITIQUESCROLL SC-7). Matching runs over the cells the
// user can actually *see* — a match scrolled out of the viewport comes back
// as an empty placeholder, so `/` can never afford a jump the eye cannot
// verify — and the both-corners probe keeps a grid's whole last row inside
// the band. The session (`searchQuery`/`searchCurrentRow`) and the jump
// (`land`) stay with the body/host; this object only answers "what text is
// on screen".
QtObject {
    id: root

    property var model: null
    property var view: null
    /// The model roles to read, in paint order (the selectable-text
    /// contract); cells for absent roles come back empty.
    property var fields: ["title"]

    /// Sparse array indexed by absolute row: visible rows carry their cell
    /// strings; off-band rows are holes. `length` is the model count, so
    /// callers keep real row indices without allocating a placeholder per
    /// row (the old dense `[""]` copy was O(model count) on every `/`).
    function rows() {
        var out = [];
        if (!root.model || !root.model.get || !root.view)
            return out;
        var v = root.view;
        var first = v.indexAt(v.contentX + Math.max(1, v.width * 0.5), v.contentY);
        var last = v.indexAt(v.contentX + Math.max(1, v.width - 1),
            v.contentY + Math.max(0, v.height - 1));
        if (first < 0)
            first = 0;
        if (last < 0)
            last = root.model.count - 1;
        out.length = root.model.count;
        for (var i = first; i <= last; ++i) {
            var item = root.model.get(i);
            var cells = [];
            for (var f = 0; f < root.fields.length; ++f) {
                var value = item[root.fields[f]];
                cells.push(value === undefined || value === null ? "" : String(value));
            }
            out[i] = cells;
        }
        return out;
    }
}
