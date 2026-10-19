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
import QtQuick.Layouts
import "Theme.js" as Theme

// The shared scroll body — SCROLLMODELANALYSIS.md is the spec.
//
// Ownership: the host owns the cursor *semantics* through `currentIndex`;
// this body owns the viewer (the active ListView/GridView, the follow, the
// memory hooks, the VI-4 selection, the `/` search fields and the adaptive
// row height); `ScrollMemory` is the only persistent state.
//
// Follow: the viewport *is* animated to the cursor's centre — a frame-driven
// glide (`followMs`, `Theme.motion.scroll`) retargeted from the live delegate
// every frame, clamped at the content ends (Neovim `'scrolloff': 0.5`
// semantics, no padding).
// `unlimitedScrolloff` opts out of the edge rule: half a viewport of empty
// space is reserved above the first row and below the last, so *every* row
// can reach the centre (the lyrics pane: the active phrase is always
// centred, even the opening and closing lines).
// This is the only follow: the highlight-band/velocity era is gone —
// `ApplyRange` enforced the band *instantly*, so a one-row move snapped
// even with a 300 ms move duration (probed; CRITIQUESCROLL SC-1).
//
// Landings: `goto(row)` (centre; the host-facing one) and `land(row)`
// (instant `Contain`, the semantic jumps: `gg`/`G`, search, restores,
// reset). **Hosts never touch `view.contentY`/`positionViewAtIndex`** —
// the body is the only writer, and `_instantFollow` shields a landing
// from the root index handler's glide.
//
// Guards: the view is a follower, never a source — its own refill/clamp
// transients are overwritten with the body's cursor; a filter/order swap
// raises `resetView()` and the payload consumes it as a top landing; an
// empty view defers the carried cursor until `onCountChanged`.
//
// ARC-UI-3: `layout` picks the active view; both share the stage, memory,
// selection, search contract and adaptive sizing. Drop it inside any
// parent Rectangle/Item (Layout or anchors).
Item {
    id: root

    // ── Model & roles ────────────────────────────────────────────────────

    property var model: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: 0
    property Component rowDelegate: null
    /// ARC-UI-3: the grid cell delegate (cover + caption, like ArtistEntry).
    property Component cellDelegate: null

    /// ARC-UI-2 M1: the model roles `/` searches, in paint order (the
    /// selectable-text contract, SEARCHACTIONSTUDY §3). Hosts override;
    /// cells for absent roles come back empty. Chrome never matches.
    property var searchFields: ["title"]
    /// ARC-UI-2 M2: the live `/` session — the query the row cells paint
    /// their blocks for, and the current candidate row (for `CurSearch`).
    /// Held by whoever owns the session (`LineSearch` pushes it down).
    property string searchQuery: ""
    property int searchCurrentRow: -1

    // ── Layout (list ⇄ grid) ─────────────────────────────────────────────

    /// ARC-UI-3: supported layouts in toggle order ("list", "grid"); a single
    /// entry means the surface never toggles.
    property var layouts: ["list"]
    /// The active body. The host sets its default; the saved layout (below)
    /// can override it on open.
    property string layout: "list"
    /// Grid geometry: columns per row and the cell height (0 = square).
    property int columns: 1
    property real cellHeight: 0
    readonly property real cellWidth: Math.max(1, root.width / Math.max(1, root.columns))

    /// ARC-UI-3: `Shift-C` on a host that declares both layouts.
    function toggleLayout() {
        if (root.layouts.length < 2)
            return;
        var i = root.layouts.indexOf(root.layout);
        root.layout = root.layouts[(i + 1) % root.layouts.length];
    }

    /// ARC-UI-3: reopen the body the user left (per `memoryKey`).
    function applySavedLayout() {
        var saved = scroll.savedLayout(root.memoryKey);
        if (saved.length > 0 && root.layouts.indexOf(saved) >= 0)
            root.layout = saved;
    }

    // ── Cursor & follow ──────────────────────────────────────────────────

    /// The cursor: the single source of truth (§1). Hosts write it; the
    /// live view mirrors it (an alias cannot follow the active view).
    property int currentIndex: -1
    onCurrentIndexChanged: {
        // A landing (`land`) owns the view: it positions and mirrors the
        // index itself; starting a glide here would fight it and leave a
        // corrupted contentY behind.
        if (root.isLanding)
            return;
        var v = root.view;
        if (v && v.count > 0 && v.currentIndex !== root.currentIndex) {
            v.currentIndex = root.currentIndex;
            root._glideTo(v, root.currentIndex);
        }
    }

    // LYR-8: the centred viewport glide — the one follow. A frame loop, not
    // a fixed NumberAnimation target: every frame derives the centre from the
    // live delegate and clamps against the current extent. Resizes, late
    // delegate realisation and contentHeight estimate growth therefore
    // retarget the glide instead of leaving it short of a stale position.
    FrameAnimation {
        id: centerGlide
        onTriggered: root._followTick()
    }

    property real _followStartY: 0
    property real _followElapsed: 0
    property int _followDuration: 0

    function _followTick() {
        var v = root.view;
        if (!v || root.currentIndex < 0 || root.isLanding) {
            centerGlide.stop();
            return;
        }
        var target = root._glideTarget(v, root.currentIndex);
        if (target === null) {
            centerGlide.stop();
            return;
        }
        root._followElapsed += centerGlide.frameTime * 1000;
        var t = root._followDuration > 0
            ? Math.min(1, root._followElapsed / root._followDuration) : 1;
        var eased = 1 - Math.pow(1 - t, 3);
        v.contentY = root._followStartY + (target - root._followStartY) * eased;
        if (t >= 1 || Math.abs(v.contentY - target) < 0.8) {
            v.contentY = target;
            centerGlide.stop();
            root.glideOverrideMs = 0;
            root._settleGlide();
        }
    }

    /// Centre the cursor over `followMs` — the delegate itself is the target
    /// (lyric rows have variable heights); an unrealised line falls back to
    /// an instant `Center`.
    function _glideTo(v, index, instant) {
        if (!v || v.count === 0 || index < 0 || root.isLanding)
            return;
        // Qt can leave contentY on a stale (too large) contentHeight
        // estimate after a landing; stop the old glide and repair it
        // before deriving the target, or the follow glides into the gap
        // under the last row.
        centerGlide.stop();
        root._clampY(v);
        var item = v.itemAtIndex(index);
        if (!item) {
            // Unrealised row (a far landing): iterative, height-aware reveal
            // (CRITIQUESCROLL SC-4/P3) — Qt's `Contain` on a scrolled list
            // parks async offsets (`-4`, `-96`, `-920`; probed) and assumes
            // fixed heights, so we step toward the row using the observed
            // local row unit instead, then glide to the centre once the row
            // exists.
            centerGlide.stop();
            root._landUnrealised(v, index);
            Qt.callLater(function () {
                if (root.currentIndex === index)
                    root._glideTo(root.view, index, instant);
            });
            return;
        }
        var target = root._glideTarget(v, index);
        if (!instant)
            root._resizeRepair = false;
        if (target === null || Math.abs(target - v.contentY) < 1)
            return;
        if (instant || Theme.reduceMotion || root.followMs <= 0) {
            v.contentY = target;
            root._settleGlide();
            return;
        }
        root._followStartY = v.contentY;
        root._followElapsed = 0;
        root._followDuration = root.glideOverrideMs > 0
            ? root.glideOverrideMs : root.followMs;
        centerGlide.stop();
        centerGlide.start();
    }

    /// The live centre target, clamped to the same bounds as `_clampY`.
    /// Only the lyrics scrolloff (opt-in, lyrics pane only) opens the range
    /// below 0, so row 0 and the last row can reach the centre; other hosts
    /// keep the old clamped-to-content behaviour. `null` when the row is not
    /// realised yet.
    function _glideTarget(v, index) {
        var item = v ? v.itemAtIndex(index) : null;
        if (!item)
            return null;
        var lo = root.unlimitedScrolloff ? Math.min(0, v.originY) : 0;
        var extent = root.contentExtent(v);
        var limit = Math.max(lo, lo + extent - v.height);
        var ideal = item.y + item.height * 0.5 - v.height * 0.5;
        return Math.max(lo, Math.min(limit, ideal));
    }

    /// Stop the glide (a manual scroll owns the viewport until the host
    /// re-follows).
    function stopGlide() {
        centerGlide.stop();
        root.glideOverrideMs = 0;
        root._resizeRepair = false;
    }

    /// Re-run the centre follow for the current cursor. `duration` overrides
    /// `followMs` for this pass — used when a layout change (a width change
    /// re-wraps variable-height rows such as lyrics) moves the cursor off
    /// centre and the host wants a quick repair, not the normal follow.
    function recenter(duration) {
        if (root.currentIndex < 0 || !root.view || root.isLanding)
            return;
        root.glideOverrideMs = Math.max(0, duration);
        root._glideTo(root.view, root.currentIndex);
        if (!centerGlide.running)
            root.glideOverrideMs = 0;
    }

    /// Resize repair: snap, never glide. A changing row unit invalidates the
    /// in-flight target, and a glide toward a stale extent can leave the row
    /// visibly stuck; the next user action resumes the normal follow.
    function recenterNow() {
        if (root.currentIndex < 0 || !root.view || root.isLanding)
            return;
        root.glideOverrideMs = 0;
        root._glideTo(root.view, root.currentIndex, true);
    }

    /// The end-of-list settle: `ListView.contentHeight` is an estimate until
    /// the bottom delegates exist, so a glide *or an instant landing* toward
    /// the last rows can stop short with the cursor under the viewport's end
    /// (`G` with variable heights left the last row cut below the fold —
    /// probed: y=1520, row bottom 1912, viewport bottom 1840). Qt's own
    /// `Contain` uses the true item geometry; it only moves the shortfall.
    function _settleGlide() {
        var v = root.view;
        if (!v || root.currentIndex < 0)
            return;
        var item = v.itemAtIndex(root.currentIndex);
        if (!item)
            return;
        if (item.y < v.contentY
                || item.y + item.height > v.contentY + v.height)
            v.positionViewAtIndex(root.currentIndex, ListView.Contain);
        root._clampY(v);
        // The per-frame contentY updates during a glide are not memory
        // writes; one coalesced save at the settle is enough.
        if (root.memoryKey)
            root._scheduleRemember();
    }

    /// The active view (itemAtIndex, positionViewAtIndex, …).
    readonly property var view: root.layout === "grid" ? grid : list
    onViewChanged: Qt.callLater(root.landCursor)
    onLayoutChanged: {
        // Only the layout field — a restore flips the layout before the
        // cursor is back, and a full save here would clobber the memory.
        if (root.memoryKey)
            scroll.rememberLayout(root.memoryKey, root.layout);
    }
    readonly property int count: root.view ? root.view.count : 0

    /// The follow duration (ms): the rows drift up/down softly as the
    /// cursor steps, and the pane fade binds to the same value. 0 snaps
    /// (`reduceMotion` also snaps, in `_glideTo`).
    property int followMs: Theme.motion.scroll
    /// One-shot duration for the next centre glide (0 = `followMs`).
    /// `recenter()` sets it for layout repairs (a width change re-wraps the
    /// lyric rows); it clears when the glide finishes or is stopped.
    property int glideOverrideMs: 0
    /// Host policy for width-triggered repair. A fixed-row list only changes
    /// its elide on width, so it stays off; grids reflow by cell size and
    /// opt in by default, while variable-row hosts (lyrics) bind their own
    /// guard (synced, active, not held).
    property bool recenterOnWidth: root.layout === "grid"
    /// Extra spacing between list rows (0 = the delegate owns it); grid
    /// cells are placed by `cellHeight`.
    property real rowSpacing: 0
    /// Fixed-row hosts can pre-calculate the content extent as
    /// `count * rowHeight` before Qt's lazy delegate estimate settles.
    /// The follow/clamp math prefers this when non-zero; otherwise
    /// ARC-UI-7.7 derives the same extent from the model count + row size.
    property real contentHeightHint: 0
    /// Unlimited scrolloff: no edge rule — the view reserves half a
    /// viewport of empty space above the first row and below the last, so
    /// the follow can centre *any* row, first and last included. The list
    /// carries header/footer spacers, the grid Flickable margins.
    property bool unlimitedScrolloff: false
    readonly property real _edgeMargin: root.unlimitedScrolloff ? root.height * 0.5 : 0

    /// ARC-UI-7.7: the extent the clamp/centre math should use. Fixed-row
    /// bodies no longer have to subscribe this themselves; variable-row
    /// hosts (lyrics) keep the live view estimate.
    function _autoContentExtent() {
        if (root.contentHeightHint > 0)
            return root.contentHeightHint;
        if (root.rowRatio <= 0 || root.count === 0)
            return 0;
        var rowH = root.layout === "grid"
            ? (root.cellHeight > 0 ? root.cellHeight : root.cellWidth)
            : root.adaptiveRowHeight;
        if (rowH <= 0)
            return 0;
        var rows = root.layout === "grid"
            ? Math.ceil(root.count / Math.max(1, root.columns))
            : root.count;
        var extent = rows * rowH;
        if (root.layout !== "grid" && root.count > 1)
            extent += (root.count - 1) * root.rowSpacing;
        if (root.unlimitedScrolloff)
            extent += 2 * root._edgeMargin;
        return extent;
    }
    function contentExtent(v) {
        if (root.contentHeightHint > 0)
            return root.contentHeightHint;
        var auto = root._autoContentExtent();
        if (auto <= 0)
            return v ? v.contentHeight : 0;
        // The last screenful is fully realised, so Qt's live contentHeight is
        // authoritative there (the derived count x row extent can run past
        // the real bottom by the page-append/rounding remainder, which made
        // `k` from `G` flick to the wrong scrolloff placement). Mid-list the
        // derived extent keeps the follow alive while Qt is still estimating;
        // the live value only grows as delegates realise.
        var live = v ? v.contentHeight : 0;
        if (live > 0) {
            var rowH = root.layout === "grid"
                ? Math.max(1, root.cellHeight > 0 ? root.cellHeight : root.cellWidth)
                : Math.max(1, root.adaptiveRowHeight);
            var tail = Math.ceil(v.height / rowH) + 1;
            // Near the end the realised rows are authoritative: their
            // accumulated per-row rounding can leave the live extent above
            // the count x unit estimate. Taking the min there clamped the
            // follow short, so the last rows could never centre (worse under
            // the adaptive squeeze, where the unit is fractional).
            if (root.currentIndex >= Math.max(0, root.count - tail))
                return live;
        }
        return auto;
    }

    /// True while a landing runs: the root index handler must not start a
    /// glide the landing is already positioning (`land` owns the view).
    property bool _instantFollow: false
    /// A resize can update the row geometry over several frames. While this
    /// is set, every contentHeight change re-snaps to the cursor instead of
    /// leaving it one relayout behind.
    property bool _resizeRepair: false
    /// ARC-UI-7.12: the landing state as a public read-only seam, so outer
    /// chrome can react without polling the private flag.
    readonly property bool isLanding: root._instantFollow

    /// Keep `contentY` inside the content bounds (Qt's async `Contain`
    /// layouts can park it outside; probed). The lyrics scrolloff is the
    /// only case with a negative lower bound (`originY` drops below 0 when
    /// its spacer header sits above row 0); every other host keeps the
    /// plain [0, contentHeight - height] range.
    function _clampY(v) {
        var lo = root.unlimitedScrolloff ? Math.min(0, v.originY) : 0;
        var extent = root.contentExtent(v);
        var limit = Math.max(lo, lo + extent - v.height);
        if (v.contentY < lo || v.contentY > limit)
            v.contentY = Math.max(lo, Math.min(limit, v.contentY));
    }

    /// Bring an unrealised row into view without Qt's `Contain`/`Center`
    /// (both park async offsets on a scrolled list — `-4`, `-96`, `-920`
    /// probed): `Beginning` anchors the row at the viewport top, we clamp
    /// the result into the content bounds, and the caller centres it. No
    /// index→pixel estimation, so variable row heights are exact
    /// (CRITIQUESCROLL SC-4/P3).
    function _landUnrealised(v, index) {
        v.positionViewAtIndex(index, ListView.Beginning);
        root._clampY(v);
    }

    /// The instant landing (semantic jumps: `gg`/`G`, search, restores, the
    /// filter reset). Positions and mirrors the cursor itself; the running
    /// glide is killed first, and the `Contain` result is repaired when Qt
    /// leaves a negative contentY on a scrolled list (probed). An empty
    /// view carries the cursor and lands it on the next count change.
    function land(index) {
        root._land(index, true);
    }

    /// The landing core. `withSettle` arms the bottom-row settle: the
    /// instant path has no glide to settle it, so the *last* row can stay
    /// cut below an under-measured contentHeight (`G` with variable
    /// heights). The payload churn (a rebuild's count climbing row by row,
    /// each intermediate end momentarily "the last row") calls this with
    /// `false` — settling every step nudged the viewport mid-refill, which
    /// was the order/type-swap flicker (#bug 2).
    function _land(index, withSettle) {
        if (index < 0)
            return;
        centerGlide.stop();
        root._resizeRepair = false;
        root._instantFollow = true;
        root.currentIndex = index;
        var v = root.view;
        if (v && v.count > 0) {
            v.currentIndex = index;
            if (v.itemAtIndex(index))
                v.positionViewAtIndex(index, ListView.Contain);
            else
                root._landUnrealised(v, index);
            root._clampY(v);
        }
        var settle = withSettle && v && v.count > 0 && index === v.count - 1;
        Qt.callLater(function () {
            root._instantFollow = false;
            if (settle)
                root._settleGlide();
        });
    }

    /// A surface returning to screen (segment switch, page pop) calls this:
    /// if the cursor is far from the viewport — or the view lost its place
    /// while hidden — it lands instantly instead of walking back.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing landing: `goto(row)` lands exactly (`Contain`) and
    /// then glides the row to the pane centre on the next tick, once the
    /// layout is true. Use `land(row)` for semantic jumps that must stay
    /// instant. **Hosts never touch `view.contentY`/`positionViewAtIndex`**
    /// — the running glide would override the teleport (CRITIQUESCROLL
    /// SC-2: the queue's current track ended up below the viewport that
    /// way).
    /// A payload rebuild (the queue's clear + refill) leaves the view empty
    /// for a beat: the follow parks in `_pendingFollow` and the next count
    /// finishes it — a skip into a not-yet-realised row must never be
    /// swallowed by the rebuild (SC-5: the double-payload skip parked the
    /// target unrealised at the clamped end, and nothing retried).
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        root._pendingFollow = i;
        root.land(i);
        Qt.callLater(function () {
            if (root.currentIndex !== i) {
                if (root._pendingFollow === i)
                    root._pendingFollow = -1; // another landing owns it now
                return;
            }
            var v = root.view;
            if (!v || v.count === 0)
                return; // `onCountChanged` finishes the follow
            root._pendingFollow = -1;
            root._glideTo(root.view, i);
        });
    }

    /// The `goto` intent that hit an empty view (see `goto`); consumed by
    /// `onCountChanged` once the payload is in.
    property int _pendingFollow: -1

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        centerGlide.stop();
        if (root.view)
            root.view.positionViewAtBeginning();
    }

    /// `gg`/`G`: instant edge landing — the follow band has nothing to
    /// animate once the target is already on screen.
    /// `gg`/`G`. VI-7: a count turns it into a line jump (`5G`/`5gg` →
    /// row 5), clamped into the list; a plain press keeps the semantic edge.
    function edgeJump(last, count) {
        if (root.count === 0)
            return;
        var n = count === undefined ? 1 : Math.max(1, count);
        if (n > 1) {
            root.land(Math.min(n, root.count) - 1);
            return;
        }
        root.land(last ? root.count - 1 : 0);
    }

    /// Shared cursor step: in grid mode the wall walks by `columns` (no
    /// wrap-around — GridView's own moveCurrentIndex* crosses rows), in list
    /// mode it is a plain ±1. VI-7: `count` steps at once, clamped at the
    /// edges (a plain step keeps returning false at the edge, so Home can
    /// hand `h` to the sidebar from the leftmost column).
    function moveStep(action, count) {
        if (root.count === 0)
            return false;
        var n = count === undefined ? 1 : Math.max(1, count);
        var cols = root.layout === "grid" ? Math.max(1, root.columns) : 1;
        var column = root.currentIndex % cols;
        var target = root.currentIndex;
        if (action === "move_left") {
            if (column === 0)
                return false;
            target -= Math.min(n, column);
        } else if (action === "move_right") {
            if (column === cols - 1 || root.currentIndex + 1 >= root.count)
                return false;
            target += Math.min(n, cols - 1 - column, root.count - 1 - root.currentIndex);
        } else if (action === "move_up") {
            if (root.currentIndex - cols * n < 0) {
                if (n === 1)
                    return false;
                target = column; // the first row's same column
            } else {
                target -= cols * n;
            }
        } else if (action === "move_down") {
            if (root.currentIndex + cols * n >= root.count) {
                if (n === 1)
                    return false;
                target = root.count - 1;
            } else {
                target += cols * n;
            }
        } else {
            return false;
        }
        if (target === root.currentIndex)
            return false;
        root.currentIndex = target;
        return true;
    }

    /// `Ctrl-D`/`Ctrl-U`: move by `fraction` of the viewport, in rows. The
    /// step reuses `moveStep` so grid mode walks whole rows and both ends
    /// clamp exactly like a counted move; the normal 300 ms follow glides.
    function pageJump(down, fraction) {
        if (root.count === 0 || !root.view)
            return false;
        var rowH = root.layout === "grid"
            ? (root.cellHeight > 0 ? root.cellHeight : root.cellWidth)
            : root.adaptiveRowHeight;
        if (rowH <= 0)
            return false;
        var rows = Math.max(1, Math.floor(
            root.view.height * (fraction === undefined ? 0.5 : fraction) / rowH));
        return root.moveStep(down ? "move_down" : "move_up", rows);
    }

    function activate() {
        if (root.currentIndex >= 0)
            root.itemActivated(root.currentIndex);
    }

    signal itemActivated(int index)

    // ── Memory (UI-54) ───────────────────────────────────────────────────

    /// Remember/restore cursor + offset under this key ("" = off).
    property string memoryKey: ""
    property int memoryScope: -1

    /// Restore the remembered cursor + offset (false when none). The saved
    /// layout is applied first so the right view holds the offset; the
    /// cursor lands instantly (a long return must not walk) and `recall`
    /// then writes the exact offset. An empty view lands the cursor on the
    /// next count change.
    function restore() {
        applySavedLayout();
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        root.land(index);
        var recalled = scroll.recall(root.memoryKey, root.view);
        // An empty view defers the landing to `onCountChanged`; the saved
        // index still owns it, so report the memory as consumed.
        return recalled.ok || root.count === 0;
    }

    function remember(scope) {
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view,
                scope === undefined ? root.memoryScope : scope, root.layout,
                root.currentIndex);
    }

    // A manual scroll emits `contentY` per pixel; coalesce the writes so a
    // flick touches the memory once per event-loop batch, not once per pixel.
    // Explicit host `remember()` calls stay immediate.
    property bool _rememberScheduled: false
    function _scheduleRemember() {
        if (!root.memoryKey || root._rememberScheduled)
            return;
        root._rememberScheduled = true;
        Qt.callLater(root._flushRemember);
    }
    function _flushRemember() {
        root._rememberScheduled = false;
        if (root.memoryKey)
            root.remember();
    }
    Component.onDestruction: root._flushRemember()

    /// §4.3: a filter/order swap rebuilds the rows under the same indices,
    /// so the next payload must land on row 0 with the offset at the top.
    /// Hosts raise this before reloading; the payload consumes it.
    property bool resetPending: false

    /// The pre-payload half of a structural swap: a true reset of the body's
    /// state *before* the model churns. The live cursor parks at -1, so the
    /// count-climb runs as a fresh list — no per-append `Contain` landing can
    /// realise stale rows (the race that left the view's window desynced from
    /// the cursor near the scrolloff edge, repaired only by a fresh `G`);
    /// the running glide and any parked follow die; the memory zeroes so no
    /// restore can chase the old spot. `resetViewNow` finishes on the payload.
    function resetView() {
        root.resetPending = true;
        centerGlide.stop();
        root._pendingFollow = -1;
        root.currentIndex = -1;
        if (root.view) {
            root.view.currentIndex = -1;
            root.view.positionViewAtBeginning();
        }
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    /// The landing itself: row 0, top, and the memory agrees (so the next
    /// restore cannot recall the old spot). Cursor and position land in the
    /// same tick, or the follow would walk the whole list back (§4.2).
    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
        root.view.positionViewAtBeginning();
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope,
                root.layout, 0);
    }

    // ── Row state (VI-4 / ARC-I-4) ───────────────────────────────────────

    // The model owns the state; hosts gate it (e.g. only when the Alt verb
    // kind exists) and keep their own cursor. The logic lives in `RowState`
    // (CRITIQUESCROLL SC-7), fed by the body's cursor.
    property bool selectable: false

    /// ARC-UI-7.9: the range changed (start/clear/all or a cursor move while
    /// a range is live), so a badge/status can follow without polling.
    signal selectionChanged

    RowState {
        id: rowState
        model: root.model
        cursor: root.currentIndex
        selectable: root.selectable
    }
    Connections {
        target: rowState
        function onSelectionChanged() { root.selectionChanged(); }
    }

    readonly property bool hasSelection: rowState.hasSelection
    readonly property int selectionCount: rowState.selectionCount

    /// `v`: start a selection on the cursor row, or end the current one.
    function toggleSelection() {
        rowState.toggle();
    }

    /// `V`: the whole list — the host moves the cursor first, this only
    /// widens the anchor.
    function selectAllRows() {
        rowState.selectAllRows();
    }

    /// Shift+motion: keep the anchor and move one step in `action`'s
    /// direction (grid-aware through `moveStep`).
    function extendSelection(action) {
        if (!rowState.extendTo(root.currentIndex))
            return false;
        return root.moveStep(action, 1);
    }

    /// Shift+Click: anchor on the old cursor, then move to `index`.
    function extendSelectionTo(index) {
        if (index < 0 || index >= root.count
                || !rowState.extendTo(root.currentIndex))
            return false;
        root.currentIndex = index;
        return true;
    }

    function clearSelection() {
        rowState.clear();
    }

    function selected(index) {
        return rowState.selected(index);
    }

    /// Ids of the selected rows ([] when there is no selection).
    function selectionIds(field) {
        return rowState.ids(field);
    }

    /// The selected rows' `starred` flags, aligned with `selectionIds`.
    function selectionStarred(field) {
        return rowState.starred(field);
    }

    /// ARC-I-4: the in-place favourite write (see `RowState`).
    function markStarred(on) {
        rowState.markStarred(on);
    }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    // The viewport-scoped row snapshot lives in `SearchContract`
    // (CRITIQUESCROLL SC-7); the session state and the jump stay here.
    SearchContract {
        id: searchContract
        model: root.model
        view: root.view
        fields: root.searchFields
    }

    /// The rendered rows the `/` prompt snapshots (see `SearchContract`).
    function searchRows() {
        return searchContract.rows();
    }

    /// Land the view cursor on a matching row; the instant landing keeps
    /// the jump from walking (the same path `gg`/`G` take).
    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.land(row);
    }

    // ── Rows (UI-58) ─────────────────────────────────────────────────────

    /// The host's nominal row fraction of the sizing height (0 = the
    /// delegates own their heights and nothing adapts).
    property real rowRatio: 0
    /// UI-58: host-supplied sizing height (0 = this body's own viewport).
    /// Page-relative hosts (the album page, the stacked search sections, the
    /// queue drawer) pass their parent Rectangle height, so inner reflows —
    /// a growing header, a sibling section appearing — never rescale rows.
    property real rowRefHeight: 0
    readonly property real rowViewHeight: root.rowRefHeight > 0
        ? root.rowRefHeight : root.height
    /// Adjustable row-height multiplier for compact hosts (Search halves it);
    /// the baseline/band still come from `Theme.adaptiveRow`.
    property real rowHeightScale: 1.0
    /// The row baseline captured on the first real measure — the default
    /// window geometry (Main starts at `Screen.height * 0.72`).
    property real _rowBase: 0
    /// The adaptive row for an explicit sizing height. Hosts whose viewport
    /// height is itself derived from the row height (the search columns snap
    /// to whole rows) call this with a stable external height, so the two
    /// cannot feed each other through `adaptiveRowHeight`.
    function adaptiveRowFor(viewHeight) {
        return Theme.adaptiveRow(viewHeight, root.rowRatio, root._rowBase)
            * root.rowHeightScale;
    }

    /// The row height to paint — scales with the sizing height inside the
    /// 18% band (0.82, 40% sooner than the original), cuts whole rows past it
    /// (see `Theme.adaptiveRow`). Delegates must size from this, never from
    /// raw viewport fractions (§5).
    readonly property real adaptiveRowHeight: adaptiveRowFor(root.rowViewHeight)

    Component.onCompleted: {
        captureRowBase();
        applySavedLayout();
    }
    // With an explicit sizing height, the baseline must come from it: a
    // content-height capture is fed by the row height the baseline itself
    // scales (`listH` → `height` → `_rowBase` → `rowH`), which is the
    // SearchColumn binding loop.
    onHeightChanged: {
        if (root.rowRefHeight <= 0)
            captureRowBase();
        root._scheduleResizeRecenter();
    }
    onRowRefHeightChanged: captureRowBase()
    onMemoryKeyChanged: applySavedLayout()

    // A resize changes the row/cell unit while `contentY` stays put, so every
    // row's y moves under the cursor. Coalesce the resize ticks, then
    // re-centre the cursor once the new layout settles. This lives in the
    // shared modal so every fixed-row host gets it, not just one tab. Width
    // is host-gated: grids reflow by cell size, lyrics re-wrap their lines,
    // while a fixed-row list only changes its elide on width.
    onWidthChanged: if (root.recenterOnWidth) root._scheduleResizeRecenter()
    onAdaptiveRowHeightChanged: root._scheduleResizeRecenter()
    onCellHeightChanged: root._scheduleResizeRecenter()
    onColumnsChanged: root._scheduleResizeRecenter()

    function _scheduleResizeRecenter() {
        if (root.currentIndex < 0)
            return;
        // A resize invalidates the glide target; keep the view still until
        // the row unit settles, then snap to the cursor.
        root.stopGlide();
        root._resizeRepair = true;
        root._glideTo(root.view, root.currentIndex, true);
        resizeRecenter.restart();
    }

    Timer {
        id: resizeRecenter
        interval: 100
        repeat: false
        onTriggered: {
            if (root.currentIndex >= 0)
                root.recenterNow();
        }
    }

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && root.rowViewHeight > 0)
            root._rowBase = root.rowViewHeight * root.rowRatio;
    }

    // Auto-fill inside Layout parents; anchor callers size it themselves.
    Layout.fillWidth: true
    Layout.fillHeight: true

    // ── The views ────────────────────────────────────────────────────────

    ScrollMemory {
        id: scroll
    }

    // Unlimited scrolloff carriers: half a viewport of empty space above the
    // first row and below the last. The list takes the spacer as a header/
    // footer (its layout ignores Flickable margins); the grid uses the
    // Flickable top/bottom margins. Either way the follow can centre row 0
    // and the final row like any other.
    Component {
        id: edgeSpacer
        Item {
            width: root.width
            height: root._edgeMargin
        }
    }

    // UI-48: the stage is the clip owner when bleed is on — it overhangs the
    // row edges by `bleed`, while both views inset by the same amount, so the
    // highlight can paint into the gutter without the real rows resizing.
    // With bleed 0 the stage is exactly the body and the view clips itself.
    Item {
        id: stage
        x: -root.bleed
        width: root.width + 2 * root.bleed
        height: root.height
        clip: true

        ListView {
            id: list
            anchors.fill: parent
            anchors.leftMargin: root.bleed
            anchors.rightMargin: root.bleed
            // With bleed the stage clips for it; otherwise the view does.
            clip: root.bleed <= 0
            visible: root.layout !== "grid"
            model: root.layout === "grid" ? null : root.model
            delegate: root.rowDelegate
            spacing: root.rowSpacing
            header: root.unlimitedScrolloff ? edgeSpacer : null
            footer: root.unlimitedScrolloff ? edgeSpacer : null
            boundsBehavior: Flickable.StopAtBounds
            // QR-9: the delegates are stateless (every visual derives from
            // model roles), so the view may pool and recycle them.
            reuseItems: true
        }

        GridView {
            id: grid
            anchors.fill: parent
            anchors.leftMargin: root.bleed
            anchors.rightMargin: root.bleed
            clip: root.bleed <= 0
            visible: root.layout === "grid"
            model: root.layout === "grid" ? root.model : null
            delegate: root.cellDelegate
            cellWidth: root.cellWidth
            cellHeight: root.cellHeight > 0 ? root.cellHeight : root.cellWidth
            // ARC-UI-7.8: GridView has no header/footer, so the unlimited
            // scrolloff rides the Flickable margins instead.
            topMargin: root.unlimitedScrolloff ? root._edgeMargin : 0
            bottomMargin: root.unlimitedScrolloff ? root._edgeMargin : 0
            boundsBehavior: Flickable.StopAtBounds
            // QR-9: same contract as the list above.
            reuseItems: true
        }
    }

    // The view side of the cursor contract (§4.1/§4.2): the active view is
    // a follower; its refill/clamp transients are never adopted.
    Connections {
        target: root.view
        function onContentYChanged() {
            // A cursor follow paints every frame; save only once it settles,
            // where `_settleGlide` schedules the coalesced remember.
            if (root.memoryKey && !centerGlide.running)
                root._scheduleRemember();
        }
        // A drag owns the viewport: kill any centre glide in flight.
        function onMovementStarted() {
            root.stopGlide();
        }
        // A shrinking contentHeight estimate (finite-row lists) can leave
        // the idle viewport past the real end; repair the gap once the
        // estimate settles, never during a glide or landing.
        function onContentHeightChanged() {
            if (centerGlide.running || root.isLanding)
                return;
            if (root._resizeRepair && root.currentIndex >= 0)
                root._glideTo(target, root.currentIndex, true);
            else
                root._clampY(target);
        }
        function onCurrentIndexChanged() {
            // A landing owns the view while it runs.
            if (target.count === 0 || root.isLanding)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            if (root.memoryKey)
                root.remember();
        }
        // A payload (re)load: a fresh list lands on row 0 (no memory, the
        // cursor was never set), an index past the new end clamps, and a
        // view that was empty a beat ago lands the carried cursor — all
        // instantly, a reload must not walk the viewport. The row-0 case is
        // the cursor-as-property replacement for the old alias accident
        // (the view defaulted to 0 and the alias silently adopted it).
        function onCountChanged() {
            if (target.count === 0)
                return;
            // Churn lands carry no settle: during a rebuild the count climbs
            // row by row and each intermediate end is momentarily the last
            // row — settling those steps nudged the viewport mid-refill
            // (the order/type-swap flicker, #bug 2).
            if (root.currentIndex < 0) {
                // A fresh list lands on row 0 — unless a memory exists, in
                // which case the host's `restore()` owns the first landing
                // (and the guarded remember below never erases it with -1).
                if (scroll.savedIndex(root.memoryKey) < 0)
                    root._land(0, false);
            } else if (root.currentIndex > target.count - 1) {
                root._land(target.count - 1, false);
            } else if (target.currentIndex !== root.currentIndex) {
                root._land(root.currentIndex, false);
            }
            if (root.memoryKey)
                root.remember();
            // A `goto` that met the empty view: the payload is in, finish
            // the follow (after the landing's `_instantFollow` release, so
            // the glide is not shielded).
            if (root._pendingFollow >= 0
                    && root._pendingFollow === root.currentIndex
                    && target.count > root._pendingFollow) {
                var i = root._pendingFollow;
                root._pendingFollow = -1;
                Qt.callLater(function () {
                    if (root.currentIndex === i && root.view && root.view.count > 0)
                        root._glideTo(root.view, i);
                });
            }
        }
    }
}
