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

// UI-54: session scroll memory. Pages save their cursor and viewport offset
// here continuously (while alive), so the values are current the moment the
// page is destroyed, and restore them on the first payload after reopening.
// `scope` optionally carries the focused pane/column the highlight was in.
// ARC-UI-3: `layout` carries the surface's active view ("list"/"grid"), so a
// reopen restores the same body the user left.
// A stale surface (an album visited weeks ago) is not worth restoring; a cap
// also keeps the serialized store bounded on large libraries.
var SCROLL_TTL_MS = 30 * 24 * 60 * 60 * 1000
var SCROLL_MAX_ENTRIES = 200

var scrollPositions = ({})

function now() {
    return Date.now()
}

/// Drops expired entries, then the oldest ones beyond the cap.
function prune() {
    var current = now()
    var keys = Object.keys(scrollPositions)
    for (var i = 0; i < keys.length; ++i) {
        var entry = scrollPositions[keys[i]]
        if (!entry || current - (entry.savedAt || 0) > SCROLL_TTL_MS)
            delete scrollPositions[keys[i]]
    }
    keys = Object.keys(scrollPositions)
    if (keys.length <= SCROLL_MAX_ENTRIES)
        return
    keys.sort(function (a, b) {
        return (scrollPositions[a].savedAt || 0)
            - (scrollPositions[b].savedAt || 0)
    })
    for (var j = 0; j < keys.length - SCROLL_MAX_ENTRIES; ++j)
        delete scrollPositions[keys[j]]
}

function saveScroll(key, index, y, scope, layout) {
    if (!key)
        return
    scrollPositions[key] = {
        index: index,
        y: y,
        scope: scope === undefined ? -1 : scope,
        layout: layout === undefined ? "" : layout,
        savedAt: now()
    }
    prune()
}

// ARC-UI-3: update only the layout field — a body restore flips the layout
// before the cursor is back, and must not rewrite the saved index/offset.
function saveLayout(key, layout) {
    if (!key)
        return
    if (scrollPositions[key] === undefined)
        scrollPositions[key] = {
            index: -1, y: 0, scope: -1, layout: layout, savedAt: now()
        }
    else {
        scrollPositions[key].layout = layout
        scrollPositions[key].savedAt = now()
    }
    prune()
}

function loadScroll(key) {
    if (!key || scrollPositions[key] === undefined)
        return null
    var entry = scrollPositions[key]
    if (now() - (entry.savedAt || 0) > SCROLL_TTL_MS) {
        delete scrollPositions[key]
        return null
    }
    return entry
}

function clearScroll(key) {
    if (key)
        delete scrollPositions[key]
}

/// ARC-UI-7.5: the whole store as JSON (settings persistence).
function serialize() {
    prune()
    return JSON.stringify(scrollPositions)
}

/// ARC-UI-7.5: merge a persisted store back in. Malformed entries are
/// ignored; entries written before `savedAt` existed get a fresh stamp so
/// the first migration does not discard them.
function hydrate(raw) {
    if (!raw)
        return
    var parsed
    try {
        parsed = JSON.parse(raw)
    } catch (error) {
        return
    }
    if (!parsed || typeof parsed !== "object")
        return
    for (var key in parsed) {
        var entry = parsed[key]
        if (!entry || typeof entry.index !== "number"
                || typeof entry.y !== "number")
            continue
        scrollPositions[key] = {
            index: entry.index,
            y: entry.y,
            scope: typeof entry.scope === "number" ? entry.scope : -1,
            layout: typeof entry.layout === "string" ? entry.layout : "",
            savedAt: typeof entry.savedAt === "number" ? entry.savedAt : now()
        }
    }
    prune()
}
