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

.pragma library

// RECICLEISLOVE.md L1a: in-place reconciliation of a `ListModel` against
// fresh payload rows, matched by `idField`. A payload swap that shares row
// identity with the model stops being a teardown:
//
//   - survivors are moved into place (`move` — the delegate lives on),
//   - changed fields are patched (`setProperty` — no rebuild, no flick),
//   - new rows are inserted at their position,
//   - gone rows are removed (duplicate overflow included).
//
// Rows without a usable id can never match: they are always removed and
// re-inserted, so an id-less payload degenerates to the old rebuild.
//
// Returns `{ kept, inserted, removed, moved, map }`; `map[oldIndex]` is the
// row's new index (-1 when it vanished) — the cursor/selection remap table.

function sync(model, rows, idField) {
    var oldIds = [];
    for (var i = 0; i < model.count; ++i)
        oldIds.push(rowId(model.get(i), idField));
    var newIds = [];
    for (var r = 0; r < rows.length; ++r)
        newIds.push(rowId(rows[r], idField));

    var stats = { kept: 0, inserted: 0, removed: 0, moved: 0, map: [] };

    // 1. Removals: ids the payload no longer wants (per occurrence — extras
    //    beyond the new multiplicity go too) and every id-less old row.
    //    Backwards so the indices stay valid.
    var wanted = {};
    for (var w = 0; w < newIds.length; ++w)
        if (newIds[w].length > 0)
            wanted[newIds[w]] = (wanted[newIds[w]] || 0) + 1;
    for (var d = model.count - 1; d >= 0; --d) {
        var id = oldIds[d];
        if (id.length > 0 && wanted[id] > 0) {
            wanted[id] -= 1;
        } else {
            model.remove(d);
            stats.removed += 1;
        }
    }

    // 2. Walk the target order: pull the next matching survivor forward
    //    (move) or insert the fresh row in place.
    for (var t = 0; t < rows.length; ++t) {
        var want = newIds[t];
        var at = want.length > 0 ? findFrom(model, want, idField, t) : -1;
        if (at < 0) {
            model.insert(t, rows[t]);
            stats.inserted += 1;
        } else {
            if (at > t) {
                model.move(at, t, 1);
                stats.moved += 1;
            }
            stats.kept += 1;
            patch(model, t, rows[t]);
        }
    }

    // 3. Tail leftovers (a survivor the walk never claimed).
    while (model.count > rows.length) {
        model.remove(model.count - 1);
        stats.removed += 1;
    }

    // 4. The remap table: old index → first unclaimed new position holding
    //    the same id (occurrence order preserved). -1 for the vanished.
    var claimed = {};
    for (var m = 0; m < oldIds.length; ++m) {
        stats.map.push(-1);
        var oldId = oldIds[m];
        if (oldId.length === 0)
            continue;
        for (var n = 0; n < newIds.length; ++n) {
            if (newIds[n] === oldId && !claimed[n]) {
                claimed[n] = true;
                stats.map[m] = n;
                break;
            }
        }
    }
    return stats;
}

/// A row's identity as a string ("" = unmatchable).
function rowId(row, idField) {
    var id = row && row[idField];
    return id === undefined || id === null ? "" : String(id);
}

/// First position at or after `from` whose id matches.
function findFrom(model, id, idField, from) {
    for (var i = from; i < model.count; ++i)
        if (rowId(model.get(i), idField) === id)
            return i;
    return -1;
}

/// Patches the changed roles only (every setProperty notifies the view).
function patch(model, index, row) {
    var current = model.get(index);
    for (var role in row) {
        if (current[role] !== row[role])
            model.setProperty(index, role, row[role]);
    }
}
