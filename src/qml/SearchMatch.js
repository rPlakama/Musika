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

// ARC-UI-2 M1: the `/` matcher — literal smartcase over each row's rendered
// cells (`[["title", "subtitle", …], …]`, in surface order). Pure JS so the
// engine is unit-testable without Qt (SEARCHACTIONSTUDY §5); no regex in v1.

/// Row indices whose cells contain the query. An uppercase character in the
/// query makes it case-sensitive (smartcase); otherwise it folds. An empty
/// query matches nothing — the caller owns the "no query, no session" rule.
function candidates(rows, query) {
    var out = []
    if (!query || query.length === 0 || !rows)
        return out
    var sensitive = /[A-Z]/.test(query)
    var needle = sensitive ? query : query.toLowerCase()
    for (var i = 0; i < rows.length; ++i) {
        var cells = rows[i]
        for (var c = 0; c < cells.length; ++c) {
            var hay = sensitive ? String(cells[c]) : String(cells[c]).toLowerCase()
            if (hay.indexOf(needle) >= 0) {
                out.push(i)
                break
            }
        }
    }
    return out
}
