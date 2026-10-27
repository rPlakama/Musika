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
import musika

// UI-54: the shared scroll-memory seam — `remember` stores the cursor +
// offset (and the focused scope, where one exists) and `recall` restores
// them for a list that was just created or reopened. The fullscreen queue,
// drawer, Main segments and both entry pages all use it.
//
// The cursor follow lives in `ScrollContent` (the glide); this object only
// persists cursor + offset (UI-54), so there is no `contentY` animation
// here to fight teleports or user input.
//
// ARC-UI-7.5: the store, TTL/cap policy and persisted JSON live in Rust.
// Writes are coalesced here (500 ms) and flushed through `Backend.scrollFlush`.
QtObject {
    id: root

    property bool _hydrated: false
    property var persistTimer: Timer {
        interval: 500
        repeat: false
        onTriggered: root.flush()
    }

    Component.onCompleted: root._ensureHydrated()
    Component.onDestruction: root.flush()

    function _ensureHydrated() {
        if (root._hydrated)
            return;
        root._hydrated = true;
        Backend.scrollHydrate();
    }

    /// Writes the Rust store to the backend's config file.
    function flush() {
        if (!root._hydrated)
            return;
        Backend.scrollFlush();
    }

    function _persist() {
        if (root._hydrated)
            root.persistTimer.restart();
    }

    /// Saves `view`'s cursor and offset under `key`; `scope` is the focused
    /// pane/column that owned the highlight, where the surface has one.
    /// ARC-UI-3: `layout` optionally carries the active body ("list"/"grid")
    /// and `index` lets the caller save its own cursor (the body's property
    /// is the source of truth while a view swap settles).
    function remember(key, view, scope, layout, index) {
        root._ensureHydrated();
        var i = index === undefined ? (view ? view.currentIndex : -1) : index;
        // A cursor of -1 is "no memory" (restore treats it that way), so a
        // teardown transient never erases a real saved position.
        if (key && view && i >= 0) {
            Backend.scrollRemember(key, i, view.contentY,
                scope === undefined ? -1 : scope,
                layout === undefined ? "" : layout);
            root._persist();
        }
    }

    /// ARC-UI-3: the remembered layout under `key` ("" when none was saved).
    function savedLayout(key) {
        root._ensureHydrated();
        return Backend.scrollSavedLayout(key);
    }

    /// The remembered cursor index under `key` (-1 when none was saved).
    function savedIndex(key) {
        root._ensureHydrated();
        return Backend.scrollSavedIndex(key);
    }

    /// ARC-UI-3: persist only the active layout.
    function rememberLayout(key, layout) {
        root._ensureHydrated();
        if (key) {
            Backend.scrollRememberLayout(key, layout);
            root._persist();
        }
    }

    /// Restores a saved cursor + offset into `view` (clamped to its current
    /// content) and guarantees the highlight is visible. Returns
    /// `{ok, index, y}` — `ok` is false when there is nothing saved or the
    /// view is empty (ARC-UI-7.5's richer recall).
    function recall(key, view) {
        root._ensureHydrated();
        var raw = Backend.scrollEntry(key);
        var saved = raw ? JSON.parse(raw) : null;
        if (!saved || !view || view.count === 0)
            return { ok: false, index: -1, y: 0 };
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
        return { ok: true, index: view.currentIndex, y: view.contentY };
    }
}
