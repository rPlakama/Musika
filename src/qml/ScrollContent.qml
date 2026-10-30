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
import QtQuick.Layouts
import QtQuick.Window
import musika
import "ScrollProfiles.js" as ScrollProfiles
import "Theme.js" as Theme

// The one scroll surface (the "ScrollModal"), list-only.
//
// The follow keeps the Neovim `scrolloff=999` feel: the cursor row stays
// centred, and the first/last screenful pins to the content edge. The
// content extent and target are solved from the row unit (count × stride),
// never from delegates that may still carry the previous geometry. The
// realised tail overrides the extent for variable-height rows.
//
// Hosts never write `contentY` or call `positionViewAtIndex` themselves.
Item {
    id: root

    property var _win: Window.window

    Connections {
        target: root._win
        function onActiveChanged() {
            if (root._win && root._win.active && root._resizeWhileInactive) {
                root._resizeWhileInactive = false;
                root._scheduleResizeRecenter();
            }
        }
    }

    // ── Context profile ──────────────────────────────────────────────────

    /// Context name selecting the metrics/flags defaults (SCROLLMODAL).
    property string context: ""
    readonly property var profile: ScrollProfiles.get(root.context)
    /// Context-only keyboard permissions (e.g. queue reorder).
    property var flags: root.profile.flags !== undefined ? root.profile.flags : ({})

    function allows(name) {
        return root.flags[name] === true;
    }

    // ── Inputs ───────────────────────────────────────────────────────────

    property var model: null
    property Component rowDelegate: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: root.profile.bleedRatio !== undefined ? root.width * root.profile.bleedRatio : 0

    /// Row sizing: nominal fraction of the reference height (0 = delegates
    /// own their heights, e.g. lyrics).
    property real rowRatio: root.profile.rowRatio !== undefined ? root.profile.rowRatio : 0
    /// The host's stable sizing reference (panel body room, page height…),
    /// never the body's own height (that would feed the row unit back into
    /// the layout).
    property real rowRefHeight: 0
    // Cached reference, not a live binding: a resize that changes the row unit
    // must settle through this stable value instead of feeding the body height
    // back into the row size.
    property real _rowRefHeight: root.height
    readonly property real rowViewHeight: root._rowRefHeight
    property real rowHeightScale: root.profile.rowHeightScale !== undefined ? root.profile.rowHeightScale : 1.0
    property real rowSpacing: 0
    /// Extra rows kept alive above and below the viewport, so a continuous
    /// scroll never has to create a delegate at the edge.
    property int renderOverscanRows: 10
    readonly property real renderOverscan: {
        var stride = root._stride(root.view);
        return stride > 0 ? stride * root.renderOverscanRows : 0;
    }
    // The Theme.adaptiveRow baseline floor is the real guard; this only stops
    // pathological 1 px rows in a tiny transient pane.
    property real minRowHeight: 18
    /// Kept for host compatibility; the extent is Qt's own now.
    property real contentHeightHint: 0
    /// A width change re-wraps variable-height rows (lyrics); let the resize
    /// repair recentre them. Lists keep the default false.
    property bool recenterOnWidth: false
    /// Animate rows whose index changed after a model move (search results).
    property bool animateDisplacement: false
    /// Bottom-align a short uniform list instead of leaving its rows at
    /// the top of an otherwise empty viewport (album pages).
    property bool bottomAlignShort: false
    /// Optional content rendered in the list header, just above the first
    /// row (a section title, for example).
    property Component headerContent: null

    /// Follow duration (ms); 0 snaps. `reduceMotion` also snaps.
    property int followMs: Theme.motion.scroll
    /// One-shot duration for the next glide (0 = `followMs`).
    property int glideOverrideMs: 0
    /// Kept for host compatibility; the follow is always centred now.
    property bool unlimitedScrolloff: false

    // ── Search / memory / selection inputs ───────────────────────────────

    property var searchFields: ["title"]
    property string searchQuery: ""
    property int searchCurrentRow: -1
    property string memoryKey: ""
    property int memoryScope: -1
    property bool selectable: false
    property bool resetPending: false

    // ── Row marks / Alt verbs ────────────────────────────────────────────

    /// ARC-UI-1: which recycled Alt-hold verb set this scroll body arms
    /// ("song"/"album"/"artist"); "" disables the overlay.
    property string quickActionKind: ""
    /// The id role a selection batch resolves to.
    readonly property string quickActionField: root.quickActionKind === "album" ? "albumId" : (root.quickActionKind === "artist" ? "artistId" : "songId")
    /// The download-window row kinds this surface renders.
    property bool markSongs: true
    property bool markAlbums: false
    /// ARC-UI-1: relayed from the body to the host (Home routes it to the
    /// shared overlay).
    signal quickAction(var target, var actions)

    // ── Row metric ───────────────────────────────────────────────────────

    /// A list with a uniform row unit (variable-height lists such as lyrics
    /// have none).
    readonly property bool adaptiveRows: root.rowRatio > 0

    function adaptiveRowFor(viewHeight) {
        if (root.rowRatio <= 0)
            return 0;
        return Theme.adaptiveRow(viewHeight, root.rowRatio, root._rowBase) * root.rowHeightScale;
    }

    /// Captured once from the first stable reference. The baseline keeps the
    /// adaptive floor steady across later height changes instead of letting
    /// the row unit chase every intermediate viewport size.
    property real _rowBase: 0

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && root.rowViewHeight > 0)
            root._rowBase = root.rowViewHeight * root.rowRatio;
    }

    readonly property real adaptiveRowHeight: Math.max(root.minRowHeight, root.adaptiveRowFor(root.rowViewHeight))
    readonly property real rowStride: root.adaptiveRows ? root.adaptiveRowHeight + root.rowSpacing : 0

    /// Native ListView header that fills the shortfall when a uniform list
    /// does not fill the viewport, pushing its rows to the bottom. The gap
    /// must come from row math (not contentHeight), or the header would feed
    /// its own height back into the calculation.
    readonly property real _shortContentGap: {
        var v = root.view;
        if (!root.bottomAlignShort || !v || v.count === 0 || v.height <= 0)
            return 0;
        if (root.rowStride <= 0)
            return 0;
        var rowsHeight = v.count * root.rowStride - root.rowSpacing;
        return Math.max(0, v.height - rowsHeight);
    }

    // ── Cursor ───────────────────────────────────────────────────────────

    property int currentIndex: -1
    /// Hosts that own the follow (lyrics) can update the highlight without
    /// recentering on every cursor change.
    property bool followCursor: true
    onCurrentIndexChanged: root._cursorMoved()
    signal itemActivated(int index)
    /// Row under a pointer, -1 outside the data rows.
    property int hoveredIndex: -1
    /// Pointer tap on a row; hosts apply their own click semantics.
    signal rowTapped(int index, bool shift)

    readonly property var view: list
    readonly property int count: root.view ? root.view.count : 0

    function rowAt(x, y) {
        var v = root.view;
        return v ? v.indexAt(x, v.contentY + y) : -1;
    }

    function refreshHovered() {
        if (!rowHover.hovered) {
            root.hoveredIndex = -1;
            return;
        }
        root.hoveredIndex = root.rowAt(rowHover.point.position.x,
            rowHover.point.position.y);
    }

    // ── Geometry helpers ─────────────────────────────────────────────────

    function _stride(v) {
        if (root.rowStride > 0)
            return root.rowStride;
        // Variable-height rows (lyrics) are measured, not derived.
        var i = root.currentIndex >= 0 ? root.currentIndex : 0;
        var item = v ? v.itemAtIndex(i) : null;
        if (item && item.height > 0)
            return item.height + root.rowSpacing;
        return 0;
    }

    /// Full rows in the viewport.
    function visibleRows() {
        var v = root.view;
        var stride = root._stride(v);
        if (!v || stride <= 0 || v.height <= 0)
            return 1;
        return Math.max(1, Math.floor(v.height / stride + 1e-6));
    }

    readonly property int visibleRowCount: root.visibleRows()

    /// First and last *item* indices in the rendered band (viewport plus
    /// overscan), used for marks and mark requests. Solved from the row unit,
    /// so a reflow does not make the mark window chase transient delegates.
    readonly property var _visibleBounds: {
        var v = root.view;
        if (!v || v.count === 0 || v.height <= 0)
            return {
                first: -1,
                last: -1
            };
        var overscan = root.renderOverscanRows;
        var stride = root._stride(v);
        if (stride <= 0)
            return {
                first: 0,
                last: v.count - 1
            };
        return {
            first: Math.max(0, Math.floor(v.contentY / stride) - overscan),
            last: Math.min(v.count - 1, Math.ceil((v.contentY + v.height) / stride) - 1 + overscan)
        };
    }
    readonly property int firstVisibleIndex: root._visibleBounds.first
    readonly property int lastVisibleIndex: root._visibleBounds.last

    function _rowOf(index) {
        return index;
    }

    function _rowCount(v) {
        return v.count;
    }

    /// The content extent from the row unit, never from delegates that may
    /// still carry the previous geometry. Variable-height rows (lyrics)
    /// consult the realised tail. `bottomAlignShort` adds its header gap.
    function _extent(v) {
        if (!v)
            return 0;
        if (root.contentHeightHint > 0)
            return root.contentHeightHint;
        var stride = root._stride(v);
        if (stride > 0 && root.rowRatio > 0)
            return Math.max(0, v.count * stride - root.rowSpacing) + root._shortContentGap;
        var last = v.count > 0 ? v.itemAtIndex(v.count - 1) : null;
        if (last && last.height > 0)
            return last.y + last.height;
        return v.contentHeight;
    }

    function _maxY(v) {
        if (!v)
            return 0;
        var maxY = Math.max(0, root._extent(v) - v.height);
        // The row-unit extent can disagree with the realised tail (spacing,
        // header gap, rounding); once the last row exists, its bottom is the
        // authority, so the final row stays glued to the viewport edge.
        var last = v.count > 0 ? v.itemAtIndex(v.count - 1) : null;
        if (last && last.height > 0) {
            var tail = last.y + last.height - v.height;
            if (tail >= 0)
                maxY = Math.min(maxY, tail);
        }
        return maxY;
    }

    /// The centred target from the row unit: pin the first/last screenful,
    /// centre only in the middle band (Neovim `scrolloff=999`). This never
    /// reads a delegate mid-reflow, so a height change cannot chase stale
    /// item positions.
    function _target(v, index) {
        if (!v || index < 0 || index >= v.count)
            return null;
        var maxY = root._maxY(v);
        var rows = root._rowCount(v);
        var row = root._rowOf(index);
        var half = root.visibleRows() * 0.5;
        if (row + 1 <= half)
            return 0;
        if (row >= rows - half)
            return maxY;
        var stride = root._stride(v);
        if (stride <= 0)
            return null;
        var center;
        if (root.rowStride > 0) {
            center = row * stride + root.adaptiveRowHeight * 0.5 - v.height * 0.5;
        } else {
            var item = v.itemAtIndex(index);
            if (item && item.height > 0)
                center = item.y + item.height * 0.5 - v.height * 0.5;
            else
                center = row * stride + (stride - root.rowSpacing) * 0.5 - v.height * 0.5;
        }
        return Math.max(0, Math.min(maxY, center));
    }

    function _clampY(v) {
        if (!v)
            return;
        var maxY = root._maxY(v);
        if (v.contentY < 0 || v.contentY > maxY)
            v.contentY = Math.max(0, Math.min(maxY, v.contentY));
    }

    /// Bring an unrealised row into view without Qt's async Contain/Center
    /// offsets: anchor it at the top, then let the follow centre it.
    function _reveal(v, index) {
        if (!v || index < 0 || index >= v.count)
            return;
        v.positionViewAtIndex(index, ListView.Beginning);
        root._clampY(v);
    }

    /// The one follow: native target, short contentY glide.
    function _follow(v, index, instant) {
        if (!v || v.count === 0 || index < 0 || index >= v.count || root._landing)
            return;
        var item = v.itemAtIndex(index);
        if (!item) {
            // A row that never realises (zero-height view, a bad delegate)
            // must not spin the callLater loop forever.
            if (root._revealTries >= 4) {
                root._clampY(v);
                return;
            }
            root._revealTries++;
            root._reveal(v, index);
            Qt.callLater(function () {
                if (root.currentIndex === index && root.view)
                    root._follow(root.view, index, instant);
            });
            return;
        }
        root._revealTries = 0;
        root._manualScroll = false;
        var target = root._target(v, index);
        if (target === null)
            return;
        var override = root.glideOverrideMs;
        root.glideOverrideMs = 0;
        glide.stop();
        if (instant || Theme.reduceMotion || root.followMs <= 0 || Math.abs(target - v.contentY) < 0.5) {
            v.contentY = target;
            root._settle();
            return;
        }
        glide.from = v.contentY;
        glide.to = target;
        glide.duration = override > 0 ? override : root.followMs;
        glide.start();
    }

    function _settle() {
        var v = root.view;
        if (!v)
            return;
        // Safety net for a row unit that disagrees with real geometry: never
        // leave the cursor fully outside the viewport.
        var item = root.currentIndex >= 0 ? v.itemAtIndex(root.currentIndex) : null;
        if (item && item.height > 0) {
            var y = v.contentY;
            if (item.y < y - 0.5)
                y = item.y;
            else if (item.y + item.height > y + v.height + 0.5)
                y = item.y + item.height - v.height;
            if (Math.abs(y - v.contentY) > 1)
                v.contentY = y;
        }
        root._clampY(v);
        if (root.memoryKey)
            root._scheduleRemember();
    }

    /// Explicit follow for hosts that turned `followCursor` off.
    function followIndex(index, instant) {
        if (root.currentIndex < 0 || !root.view)
            return;
        root._manualScroll = false;
        root._follow(root.view, index, instant === true);
    }

    function stopGlide() {
        glide.stop();
        root.glideOverrideMs = 0;
    }

    /// Re-run the follow for the current cursor over `duration` ms.
    function recenter(duration) {
        if (root.currentIndex < 0 || !root.view)
            return;
        root._manualScroll = false;
        root.glideOverrideMs = Math.max(0, duration);
        root._follow(root.view, root.currentIndex, false);
    }

    /// Snap the cursor to its native centred position now.
    function recenterNow() {
        if (root.currentIndex < 0 || !root.view)
            return;
        root._manualScroll = false;
        root.glideOverrideMs = 0;
        root._follow(root.view, root.currentIndex, true);
    }

    function _cursorMoved() {
        var teleport = root._teleportMove;
        root._teleportMove = false;
        if (root._landing)
            return;
        root._manualScroll = false;
        var v = root.view;
        if (!v)
            return;
        if (root.currentIndex < 0) {
            if (v.currentIndex !== -1)
                v.currentIndex = -1;
            return;
        }
        if (v.count === 0)
            return;
        var index = Math.min(v.count - 1, root.currentIndex);
        if (index !== root.currentIndex) {
            root.currentIndex = index;
            return;
        }
        if (v.currentIndex !== index)
            v.currentIndex = index;
        if (!root.followCursor) {
            root._settle();
            return;
        }
        root._follow(v, index, teleport);
    }

    /// Instant landing (semantic jumps: gg/G, search, restores, reset).
    function land(index) {
        root._land(index, true);
    }

    function _land(index, withSettle) {
        if (index < 0)
            return;
        root._manualScroll = false;
        var v = root.view;
        if (v && v.count > 0)
            index = Math.max(0, Math.min(index, v.count - 1));
        glide.stop();
        root._modelReset = false;
        root._landing = true;
        root._revealTries = 0;
        root.currentIndex = index;
        var unrealised = v && v.count > 0 && !v.itemAtIndex(index);
        if (v && v.count > 0) {
            v.currentIndex = index;
            if (unrealised)
                root._reveal(v, index);
            var target = root._target(v, index);
            if (target !== null)
                v.contentY = target;
            root._clampY(v);
        }
        root._landing = false;
        Qt.callLater(function () {
            var w = root.view;
            if (!w || w.count === 0 || root.currentIndex !== index)
                return;
            if (withSettle && unrealised) {
                // The anchor only got the row on screen; centre it now that
                // it exists (a memory restore owns its own offset).
                root._follow(w, index, true);
            } else if (withSettle) {
                root._settle();
            }
        });
    }

    /// A surface returning to screen calls this: land on the cursor.
    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    /// The host-facing jump: force a follow even when the index already
    /// matches (a manual scroll may have moved the viewport).
    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        if (root.count > 0)
            i = Math.max(0, Math.min(i, root.count - 1));
        root._manualScroll = false;
        root._pendingFollow = i;
        root.currentIndex = i;
        var v = root.view;
        if (!v || v.count === 0)
            return;
        if (!v.itemAtIndex(i)) {
            root._land(i, false);
            return;
        }
        root._pendingFollow = -1;
        root._follow(v, i, false);
    }

    property int _pendingFollow: -1
    property bool _countRepairScheduled: false
    property bool _modelReset: false

    function _scheduleCountRepair() {
        if (root._countRepairScheduled)
            return;
        root._countRepairScheduled = true;
        countRepairTimer.restart();
    }

    function _repairCount() {
        root._countRepairScheduled = false;
        var v = root.view;
        if (!v || v.count === 0 || root.resetPending)
            return;
        if (root._landing) {
            root._scheduleCountRepair();
            return;
        }
        var rebuilt = root._modelReset;
        root._modelReset = false;
        if (rebuilt && root.currentIndex >= 0)
            root._land(root.currentIndex, false);
        else if (root.currentIndex < 0) {
            if (scroll.savedIndex(root.memoryKey) < 0)
                root._land(0, false);
        } else if (root.currentIndex > v.count - 1)
            root._land(v.count - 1, false);
        else if (v.currentIndex !== root.currentIndex)
            root._land(root.currentIndex, false);
        if (root.memoryKey)
            root.remember();
        if (root._pendingFollow >= v.count)
            root._pendingFollow = -1;
        if (root._pendingFollow >= 0 && root._pendingFollow === root.currentIndex && v.count > root._pendingFollow) {
            var i = root._pendingFollow;
            root._pendingFollow = -1;
            Qt.callLater(function () {
                if (root.currentIndex === i && root.view && root.view.count > 0)
                    root._follow(root.view, i, false);
            });
        }
    }

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        glide.stop();
        if (root.view)
            root.view.positionViewAtBeginning();
    }

    /// `gg`/`G`: instant landing. A count turns it into a line jump.
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

    /// Shared cursor step: a plain ±1 row (or `count` rows), clamped.
    function moveStep(action, count) {
        if (root.count === 0)
            return false;
        var n = count === undefined ? 1 : Math.max(1, count);
        var target = root.currentIndex;
        if (action === "move_up") {
            if (root.currentIndex - n < 0) {
                if (n === 1)
                    return false;
                target = 0;
            } else {
                target -= n;
            }
        } else if (action === "move_down") {
            if (root.currentIndex + n >= root.count) {
                if (n === 1)
                    return false;
                target = root.count - 1;
            } else {
                target += n;
            }
        } else {
            return false;
        }
        if (target === root.currentIndex)
            return false;
        // VI-7: a big count is a teleport, not a long scroll animation.
        root._teleportMove = n > root.teleportThreshold;
        root.currentIndex = target;
        return true;
    }

    /// `Ctrl-D`/`Ctrl-U`: move by `fraction` of the viewport, in rows.
    function pageJump(down, fraction) {
        if (root.count === 0 || !root.view)
            return false;
        var rowH = root.adaptiveRowHeight;
        if (rowH <= 0)
            rowH = root._stride(root.view);
        if (rowH <= 0)
            return false;
        var rows = Math.max(1, Math.floor(root.view.height * (fraction === undefined ? 0.5 : fraction) / rowH));
        return root.moveStep(down ? "move_down" : "move_up", rows);
    }

    function activate() {
        if (root.currentIndex >= 0)
            root.itemActivated(root.currentIndex);
    }

    // ── Memory (UI-54) ───────────────────────────────────────────────────

    function restore() {
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        if (root.count > 0)
            index = Math.max(0, Math.min(index, root.count - 1));
        root._land(index, false);
        var recalled = scroll.recall(root.memoryKey, root.view);
        return recalled.ok || root.count === 0;
    }

    function remember(scope) {
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, scope === undefined ? root.memoryScope : scope, "list", root.currentIndex);
    }

    property bool _rememberScheduled: false
    function _scheduleRemember() {
        if (!root.memoryKey || root._rememberScheduled)
            return;
        root._rememberScheduled = true;
        rememberTimer.restart();
    }
    function _flushRemember() {
        root._rememberScheduled = false;
        if (root.memoryKey)
            root.remember();
    }
    Component.onDestruction: root._flushRemember()

    // ── Resize ───────────────────────────────────────────────────────────

    property bool _manualScroll: false
    property bool _landing: false
    property int _revealTries: 0
    /// A numeric move above `teleportThreshold` lands natively instead of
    /// gliding through every intermediate row.
    property bool _teleportMove: false
    /// The row count above which a numeric move teleports (`15` = 16+).
    property int teleportThreshold: 15
    property bool _resizeScheduled: false
    property bool _resizing: false
    property bool _resizeApplyPending: false
    /// A height change while the window is inactive can land before Qt has
    /// laid the view out; finish the repair once the window is active again.
    property bool _resizeWhileInactive: false
    /// The visual top row while the user (not the cursor) owns the viewport.
    /// A resize restores this row instead of yanking the view back to the
    /// keyboard cursor.
    property int _anchorRow: 0
    property real _anchorFraction: 0

    /// Remember the visual top row when the user owns the viewport.
    function _captureAnchor() {
        var v = root.view;
        if (!v || v.count === 0)
            return;
        var stride = root._stride(v);
        if (stride <= 0)
            return;
        root._anchorRow = Math.max(0, Math.min(v.count - 1, Math.floor(v.contentY / stride)));
        root._anchorFraction = Math.max(0, Math.min(0.999, (v.contentY - root._anchorRow * stride) / stride));
    }

    /// A resize keeps the keyboard cursor centred when the follow owns the
    /// view; after a manual scroll it keeps the visual top row instead. This
    /// writes contentY directly: native positioning during the relayout is
    /// what left the viewport one layout behind.
    function _applyResizeAnchor() {
        if (root._landing)
            return;
        var v = root.view;
        if (!v || v.count === 0 || v.height <= 0)
            return;
        var target = null;
        if (!root._manualScroll && root.currentIndex >= 0 && root.currentIndex < v.count)
            target = root._target(v, root.currentIndex);
        if (target === null) {
            var s = root._stride(v);
            if (s > 0) {
                var anchor = Math.max(0, Math.min(v.count - 1, root._anchorRow));
                target = (anchor + root._anchorFraction) * s;
            }
        }
        if (target !== null)
            v.contentY = Math.max(0, Math.min(root._maxY(v), target));
        root._ensureCursorVisible(v);
    }

    /// A manual scroll can own the top anchor while the resize leaves the
    /// keyboard cursor outside the viewport. If that happened, put the cursor
    /// back on its centred/pinned row; keep the manual anchor when the cursor
    /// is still visible.
    function _ensureCursorVisible(v) {
        if (!v || root.currentIndex < 0 || root.currentIndex >= v.count)
            return;
        var item = v.itemAtIndex(root.currentIndex);
        if (item && item.height > 0
                && item.y >= v.contentY - 0.5
                && item.y + item.height <= v.contentY + v.height + 0.5)
            return;
        var target = root._target(v, root.currentIndex);
        if (target !== null)
            v.contentY = Math.max(0, Math.min(root._maxY(v), target));
    }

    /// One deferred pass per resize batch, off the compositor event. Wait a
    /// frame and then 100 ms for the row unit to settle; snapping on every
    /// intermediate height event is what broke the scroll during an
    /// application-window resize.
    function _scheduleResizeRecenter() {
        if (!root.view)
            return;
        root.stopGlide();
        root._resizing = true;
        if (Window.window && !Window.window.active)
            root._resizeWhileInactive = true;
        if (!root._resizeApplyPending) {
            root._resizeApplyPending = true;
            Qt.callLater(root._applyPendingResize);
        }
        root._resizeScheduled = true;
        resizeTimer.restart();
    }

    function _applyPendingResize() {
        root._resizeApplyPending = false;
        root._applyResizeAnchor();
    }

    function _resizeSettle() {
        root._resizeScheduled = false;
        root._applyResizeAnchor();
        root._resizing = false;
    }

    // Timers, not Qt.callLater: a Loader can destroy this ScrollContent
    // between scheduling and delivery, and a destroyed object must not try to
    // evaluate a queued function in its dead context.
    Timer {
        id: resizeTimer
        interval: 100
        repeat: false
        onTriggered: root._resizeSettle()
    }
    Timer {
        id: countRepairTimer
        interval: 0
        repeat: false
        onTriggered: root._repairCount()
    }
    Timer {
        id: rememberTimer
        interval: 0
        repeat: false
        onTriggered: root._flushRemember()
    }

    Component.onCompleted: {
        root._rowRefHeight = root.rowRefHeight > 0 ? root.rowRefHeight : root.height;
        root.captureRowBase();
    }
    onRowRefHeightChanged: {
        root._rowRefHeight = root.rowRefHeight > 0 ? root.rowRefHeight : root.height;
        root.captureRowBase();
    }
    onWidthChanged: if (root.recenterOnWidth)
        root._scheduleResizeRecenter()
    onHeightChanged: {
        if (root.rowRefHeight <= 0)
            root._rowRefHeight = root.height;
        root.captureRowBase();
        root._scheduleResizeRecenter();
    }
    onAdaptiveRowHeightChanged: root._scheduleResizeRecenter()

    // ── Selection (VI-4 / ARC-I-4) ───────────────────────────────────────

    signal selectionChanged

    RowState {
        id: rowState
        model: root.model
        cursor: root.currentIndex
        selectable: root.selectable
    }
    Connections {
        target: rowState
        function onSelectionChanged() {
            root.selectionChanged();
        }
    }

    // The recycled Alt verb sets and the one download-mark window. Both are
    // cursor/selection concerns, so the body owns them once for every host.
    QuickActions {
        id: quickVerbs
    }

    MarkWindow {
        id: markWindow
        model: root.model
        firstVisible: root.firstVisibleIndex
        lastVisible: root.lastVisibleIndex
        cursor: root.currentIndex
        songs: root.markSongs
        albums: root.markAlbums
    }

    function requestMarkWindow(force) {
        markWindow.request(force === true);
    }

    /// ARC-UI-1: arm the Alt overlay on the cursor row (or the selection's
    /// batch verbs). The host routes the emitted actions to the shared
    /// overlay; the optimistic heart write keeps the rows in step.
    function armQuickActions() {
        if (root.quickActionKind === "" || root.count === 0 || root.currentIndex < 0)
            return;
        var item = root.model ? root.model.get(root.currentIndex) : null;
        if (!item)
            return;

        var actions = null;
        if (root.hasSelection) {
            actions = quickVerbs.forSelection(root.quickActionKind, root.selectionIds(root.quickActionField), root.selectionStarred(root.quickActionField));
        } else {
            var preview = function (level) {
                if (root.model && root.model.setProperty && item.rating !== undefined)
                    root.model.setProperty(root.currentIndex, "rating", level);
            };
            actions = quickVerbs.forItem(root.quickActionKind, item, Backend.cachedAlbumIds.indexOf(item.albumId) !== -1, preview);
        }
        if (!actions)
            return;

        // ARC-I-4: the heart tracks the verb immediately — the cursor row or
        // the whole selected range is written in place, no reload.
        quickVerbs.wrapFavorite(actions, function (on) {
            rowState.markStarred(on);
        });

        var row = root.view ? root.view.itemAtIndex(root.currentIndex) : null;
        if (row)
            root.quickAction(row, actions);
    }

    readonly property bool hasSelection: rowState.hasSelection
    readonly property int selectionCount: rowState.selectionCount

    function toggleSelection() {
        rowState.toggle();
    }
    function selectAllRows() {
        rowState.selectAllRows();
    }
    function extendSelection(action) {
        if (!rowState.extendTo(root.currentIndex))
            return false;
        return root.moveStep(action, 1);
    }
    function extendSelectionTo(index) {
        if (index < 0 || index >= root.count || !rowState.extendTo(root.currentIndex))
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
    function selectionIds(field) {
        return rowState.ids(field);
    }
    function selectionIndices() {
        return rowState.indices();
    }
    function selectionStarred(field) {
        return rowState.starred(field);
    }
    function markStarred(on) {
        rowState.markStarred(on);
    }

    // ── Search (ARC-UI-2) ────────────────────────────────────────────────

    SearchContract {
        id: searchContract
        model: root.model
        view: root.view
        fields: root.searchFields
    }

    function searchRows() {
        return searchContract.rows();
    }
    function searchJump(row) {
        if (row < 0 || row >= root.count)
            return;
        root.land(row);
    }

    // ── Reset (UI-10) ────────────────────────────────────────────────────

    function resetView() {
        root.resetPending = true;
        glide.stop();
        root._pendingFollow = -1;
        root._manualScroll = false;
        root.currentIndex = -1;
        if (root.view) {
            root.view.currentIndex = -1;
            root.view.positionViewAtBeginning();
        }
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope, "list", 0);
    }

    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope, "list", 0);
    }

    // ── The views ────────────────────────────────────────────────────────

    Layout.fillWidth: true
    Layout.fillHeight: true

    ScrollMemory {
        id: scroll
    }

    NumberAnimation {
        id: glide
        target: root.view
        property: "contentY"
        easing.type: Easing.OutCubic
        onFinished: root._settle()
    }

    Transition {
        id: displacedTransition
        Anim {
            properties: "x,y"
            duration: Theme.motion.standard
            easing.bezierCurve: Theme.ease.decelerate
        }
    }

    // UI-48: the stage is the clip owner when bleed is on.
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
            clip: root.bleed <= 0
            model: root.model
            delegate: root.rowDelegate
            spacing: root.rowSpacing
            header: Column {
                width: list.width
                Item {
                    width: parent.width
                    height: Math.max(0, root._shortContentGap - headerLoader.height)
                }
                Loader {
                    id: headerLoader
                    width: parent.width
                    sourceComponent: root.headerContent
                }
            }
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
            reuseItems: !root.animateDisplacement
            displaced: root.animateDisplacement ? displacedTransition : null
            move: root.animateDisplacement ? displacedTransition : null
            moveDisplaced: root.animateDisplacement ? displacedTransition : null
            cacheBuffer: root.renderOverscan

            HoverHandler {
                id: rowHover
                cursorShape: root.hoveredIndex >= 0
                    ? Qt.PointingHandCursor : Qt.ArrowCursor
                onPointChanged: root.refreshHovered()
                onHoveredChanged: root.refreshHovered()
            }

            TapHandler {
                acceptedButtons: Qt.LeftButton
                acceptedModifiers: Qt.KeyboardModifierMask
                onTapped: {
                    var index = root.rowAt(point.position.x, point.position.y);
                    if (index >= 0)
                        root.rowTapped(index,
                            (point.modifiers & Qt.ShiftModifier) !== 0);
                }
            }
        }
    }

    Connections {
        target: root.view
        function onContentYChanged() {
            root.refreshHovered();
            if (root._manualScroll && !glide.running && !root._landing)
                root._captureAnchor();
            if (root.memoryKey && !glide.running)
                root._scheduleRemember();
        }
        function onMovementStarted() {
            root.stopGlide();
            root._manualScroll = true;
        }
        function onContentHeightChanged() {
            // The resize pass owns the viewport until its 100 ms settle; do
            // not fight it from intermediate contentHeight updates. While a
            // glide runs the target absorbs the shift.
            if (root._landing || root._resizing || root._resizeApplyPending)
                return;
            if (!glide.running)
                root._clampY(target);
        }
        function onCurrentIndexChanged() {
            if (target.count === 0)
                return;
            if (root.currentIndex >= 0 && target.currentIndex !== root.currentIndex)
                target.currentIndex = root.currentIndex;
            if (root.memoryKey)
                root.remember();
        }
        function onCountChanged() {
            if (target.count === 0) {
                root._modelReset = true;
                return;
            }
            root._scheduleCountRepair();
        }
    }
}
