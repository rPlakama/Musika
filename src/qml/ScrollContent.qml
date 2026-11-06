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

pragma ComponentBehavior: Bound

import QtQuick
import musika
import "ScrollProfiles.js" as ScrollProfiles
import "Theme.js" as Theme

// The one scroll surface (the "ScrollModal"), list-only.
//
// One behaviour: Neovim `scrolloff=999`. The cursor row is centred whenever
// the row-unit target can stay inside the content; near the first and last
// rows the target clamps to the content edge, so the edge rows touch the
// viewport and never leave a gap.
//
// The view is a pure function of the cursor:
//
//   centre(i) = i * stride + rowHeight / 2 - viewHeight / 2
//   targetY(i) = clamp(centre(i), minY, maxY)
//
// Uniform rows are solved from count * stride only. Delegate positions,
// contentHeight and timers are never consulted for them. Variable-height
// hosts (lyrics) keep a measured item.y fallback, because their rows cannot
// use the closed form.
Item {
    id: root

    // ── Context profile ──────────────────────────────────────────────────

    /// Context name selecting the metrics/flags defaults (SCROLLMODAL).
    property string context: ""
    readonly property var profile: ScrollProfiles.get(root.context)
    /// Context-only keyboard permissions (e.g. queue reorder).
    property var flags: root.profile.flags !== undefined ? root.profile.flags : ({})

    function allows(name) {
        return root.flags[name] === true;
    }

    // ── Removable type inserts ─────────────────────────────────────────
    // A host declares the scopes this surface may render; the array is
    // mutable, so a page can drop or add a type without changing the scroll
    // engine. `limitedBy` names the enclosing scope (empty = global).
    property var types: []
    property string limitedBy: ""
    property string activeType: ""
    readonly property bool hasTypeScope: root.types.length > 0
    signal typeActivated(string type)

    function setType(type) {
        if (root.activeType === type || root.types.indexOf(type) < 0)
            return false;
        root.activeType = type;
        root.typeActivated(type);
        return true;
    }

    // ── Inputs ───────────────────────────────────────────────────────────

    property var model: null
    /// Rows may come from a ListModel or the count-only RowWindow.
    property var rowAccess: null
    /// Scope surfaces suppress the released-delegate cache; other lists keep it.
    property bool reuseRows: true
    readonly property var access: root.rowAccess ? root.rowAccess : root.model
    property Component rowDelegate: null
    /// Gutter bleed on each side (UI-48); 0 clips at the list edge.
    property real bleed: root.profile.bleedRatio !== undefined ? root.width * root.profile.bleedRatio : 0

    /// Row sizing: nominal fraction of the reference height (0 = delegates
    /// own their heights, e.g. lyrics).
    property real rowRatio: root.profile.rowRatio !== undefined ? root.profile.rowRatio : 0
    /// The host's stable sizing reference (panel body room, page height…),
    /// never the body's own height when the host can provide one.
    property real rowRefHeight: 0
    readonly property real rowViewHeight: root.rowRefHeight > 0 ? root.rowRefHeight : root.height
    property real rowHeightScale: root.profile.rowHeightScale !== undefined ? root.profile.rowHeightScale : 1.0
    property real rowSpacing: 0
    /// Extra rows kept alive above and below the viewport.
    property int renderOverscanRows: 10
    readonly property real renderOverscan: {
        var stride = root.rowStride > 0 ? root.rowStride : root.rowViewHeight;
        return stride > 0 ? stride * root.renderOverscanRows : 0;
    }
    property real minRowHeight: 18
    /// Animate rows whose index changed after a model move (search results).
    property bool animateDisplacement: false
    /// Keep a short uniform list against the bottom edge by growing the
    /// native header. No special target math is needed.
    property bool bottomAlignShort: false
    /// Optional content rendered in the list header, above the first row.
    property Component headerContent: null

    /// Follow duration (ms); 0 snaps. `reduceMotion` also snaps.
    property int followMs: Theme.motion.scroll
    /// Debug-only: log the follow residual after a glide finishes.
    property bool debugFollow: false

    // ── Search / memory / selection inputs ───────────────────────────────

    property var searchFields: ["title"]
    property string searchQuery: ""
    property int searchCurrentRow: -1
    property string memoryKey: ""
    property int memoryScope: -1
    property bool selectable: false
    property bool resetPending: false

    // ── Row marks / Alt verbs ────────────────────────────────────────────

    property string quickActionKind: ""
    readonly property string quickActionField: root.quickActionKind === "album" ? "albumId" : (root.quickActionKind === "artist" ? "artistId" : "songId")
    property bool markSongs: true
    property bool markAlbums: false
    signal quickAction(var target, var actions)

    // ── Row metric ───────────────────────────────────────────────────────

    readonly property bool adaptiveRows: root.rowRatio > 0

    function adaptiveRowFor(viewHeight) {
        if (root.rowRatio <= 0)
            return 0;
        return Theme.adaptiveRow(viewHeight, root.rowRatio, root._rowBase) * root.rowHeightScale;
    }

    /// Captured once from the first stable reference. It is a metric, not
    /// interactive state; resize never recaptures it.
    property real _rowBase: 0

    function captureRowBase() {
        if (root._rowBase <= 0 && root.rowRatio > 0 && root.rowViewHeight > 0)
            root._rowBase = root.rowViewHeight * root.rowRatio;
    }

    // Qt's ListView content coordinate is integer-based; a fractional
    // delegate height makes contentHeight/originY drift and clamps the top.
    readonly property real adaptiveRowHeight: Math.max(root.minRowHeight,
        Math.round(root.adaptiveRowFor(root.rowViewHeight)))
    readonly property real rowStride: root.adaptiveRows ? root.adaptiveRowHeight + root.rowSpacing : 0

    /// The header gap that pushes a short uniform list to the bottom.
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
    /// recentering on every cursor change. Explicit follow functions still
    /// work.
    property bool followCursor: true
    onCurrentIndexChanged: {
        if (root.followCursor)
            root.sync(true);
        if (root.memoryKey)
            root.remember();
    }
    signal itemActivated(int index)
    property int hoveredIndex: -1
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

    // ── The one geometry contract ────────────────────────────────────────

    /// Qt's content edge above the first row (negative when a header exists).
    readonly property real minY: root.headerHeight > 0 ? -root.headerHeight : 0
    readonly property real headerHeight: root.view && root.view.headerItem
        ? root.view.headerItem.height : 0
    readonly property real footerHeight: root.view && root.view.footerItem
        ? root.view.footerItem.height : 0

    /// The furthest contentY that still keeps the last row at the viewport
    /// bottom. Uniform rows use the closed form; variable rows keep Qt's
    /// measured extent.
    readonly property real maxY: {
        var v = root.view;
        if (!v || root.rowStride <= 0)
            return v ? Math.max(root.minY, v.contentHeight - v.height) : root.minY;
        var extent = v.count * root.rowStride - root.rowSpacing;
        return Math.max(root.minY, extent + root.footerHeight - v.height);
    }

    /// The row-unit target. `null` only for an unmeasurable variable row.
    function targetY(index) {
        var v = root.view;
        if (!v || index < 0 || index >= v.count)
            return root.minY;
        if (root.rowStride > 0) {
            var center = index * root.rowStride + root.adaptiveRowHeight * 0.5
                - v.height * 0.5;
            return Math.max(root.minY, Math.min(root.maxY, center));
        }
        // Variable-height rows cannot use the closed form. This is the
        // measured fallback for the lyrics pane; it is intentionally the
        // only place that reads a delegate.
        var item = v.itemAtIndex(index);
        if (!item || item.height <= 0)
            return root.minY;
        var measured = item.y + item.height * 0.5 - v.height * 0.5;
        return Math.max(root.minY, Math.min(root.maxY, measured));
    }

    /// The only writer of contentY. Pure and idempotent: calling it again, or
    /// after any model/layout change, converges on targetY(currentIndex).
    /// `forceY` is used only by resetTop, which intentionally leaves the
    /// cursor alone.
    function sync(animated, forceY) {
        var v = root.view;
        if (!v)
            return;
        var y = forceY !== undefined ? forceY : root.targetY(root.currentIndex);
        if (animated === true && !Theme.reduceMotion && root.followMs > 0
                && Math.abs(y - v.contentY) <= 2 * v.height) {
            glide.stop();
            glide.from = v.contentY;
            glide.to = y;
            glide.duration = root._oneShotMs > 0 ? root._oneShotMs : root.followMs;
            glide.start();
        } else {
            glide.stop();
            root._oneShotMs = 0;
            v.contentY = y;
        }
        root.updateVisibleBounds();
    }

    function stopGlide() {
        glide.stop();
        root._oneShotMs = 0;
    }

    function followIndex(index, instant) {
        if (index >= 0)
            root.currentIndex = index;
        root.sync(instant !== true);
    }

    function recenter(duration) {
        root._oneShotMs = Math.max(0, duration === undefined ? 0 : duration);
        root.sync(true);
    }

    function recenterNow() {
        root._oneShotMs = 0;
        root.sync(false);
    }

    function land(index) {
        if (index < 0)
            return;
        if (root.count > 0)
            index = Math.max(0, Math.min(index, root.count - 1));
        glide.stop();
        root.currentIndex = index;
        root.sync(false);
    }

    function landCursor() {
        if (root.currentIndex >= 0)
            root.land(root.currentIndex);
    }

    function goto(index) {
        var i = index === undefined ? root.currentIndex : index;
        if (i < 0)
            return;
        if (root.count > 0)
            i = Math.max(0, Math.min(i, root.count - 1));
        root.currentIndex = i;
        root.sync(false);
    }

    /// Top the viewport without touching the cursor (a fresh payload).
    function resetTop() {
        root.stopGlide();
        root.sync(false, root.minY);
    }

    // ── Visible band ─────────────────────────────────────────────────────

    function visibleRows() {
        var v = root.view;
        var stride = root.rowStride;
        if (!v || stride <= 0 || v.height <= 0)
            return 1;
        return Math.max(1, Math.floor(v.height / stride + 1e-6));
    }

    readonly property int visibleRowCount: root.visibleRows()
    property int firstVisibleIndex: -1
    property int lastVisibleIndex: -1

    /// Int-only publication: no object is allocated per scroll frame, and
    /// the properties only change when the band actually moves.
    function updateVisibleBounds() {
        var v = root.view;
        if (!v || v.count === 0 || v.height <= 0) {
            if (root.firstVisibleIndex !== -1)
                root.firstVisibleIndex = -1;
            if (root.lastVisibleIndex !== -1)
                root.lastVisibleIndex = -1;
            return;
        }
        var first = 0;
        var last = v.count - 1;
        if (root.rowStride > 0) {
            var top = Math.max(0, v.contentY - root.minY - root.headerHeight);
            first = Math.max(0, Math.floor(top / root.rowStride));
            last = Math.min(v.count - 1,
                Math.ceil((top + v.height) / root.rowStride) - 1);
        }
        first = Math.max(0, first - root.renderOverscanRows);
        last = Math.min(v.count - 1, last + root.renderOverscanRows);
        if (first !== root.firstVisibleIndex)
            root.firstVisibleIndex = first;
        if (last !== root.lastVisibleIndex)
            root.lastVisibleIndex = last;
        if (rowHover.hovered)
            root.refreshHovered();
    }

    // ── Movement API ─────────────────────────────────────────────────────

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
        root.currentIndex = target;
        return true;
    }

    function pageJump(down, fraction) {
        if (root.count === 0 || !root.view)
            return false;
        var rowH = root.rowStride > 0 ? root.rowStride : root.adaptiveRowHeight;
        if (rowH <= 0)
            return false;
        var rows = Math.max(1,
            Math.floor(root.view.height * (fraction === undefined ? 0.5 : fraction) / rowH));
        return root.moveStep(down ? "move_down" : "move_up", rows);
    }

    function activate() {
        if (root.currentIndex >= 0)
            root.itemActivated(root.currentIndex);
    }

    // ── Memory (UI-54) ───────────────────────────────────────────────────

    /// Only the cursor is semantic; contentY is derived by sync().
    function remember(scope) {
        if (!root.memoryKey || !root.view || root.currentIndex < 0)
            return;
        scroll.remember(root.memoryKey, root.view,
            scope === undefined ? root.memoryScope : scope, "list",
            root.currentIndex);
    }

    function restore() {
        var index = scroll.savedIndex(root.memoryKey);
        if (index < 0)
            return false;
        if (root.count > 0)
            index = Math.max(0, Math.min(index, root.count - 1));
        root.currentIndex = index;
        root.sync(false);
        return true;
    }

    function resetView() {
        root.resetPending = true;
        root.stopGlide();
        root.currentIndex = -1;
        root.sync(false, root.minY);
        if (root.memoryKey)
            scroll.remember(root.memoryKey, root.view, root.memoryScope, "list", -1);
    }

    function resetViewNow() {
        if (root.count === 0)
            return;
        root.land(0);
    }

    Component.onDestruction: root.remember()
    Component.onCompleted: {
        root.captureRowBase();
        root.sync(false);
    }

    // ── Selection (VI-4 / ARC-I-4) ───────────────────────────────────────

    signal selectionChanged

    RowState {
        id: rowState
        model: root.access
        cursor: root.currentIndex
        selectable: root.selectable
    }
    Connections {
        target: rowState
        function onSelectionChanged() {
            root.selectionChanged();
        }
    }

    QuickActions {
        id: quickVerbs
    }

    MarkWindow {
        id: markWindow
        model: root.access
        firstVisible: root.firstVisibleIndex
        lastVisible: root.lastVisibleIndex
        cursor: root.currentIndex
        songs: root.markSongs
        albums: root.markAlbums
    }

    function requestMarkWindow(force) {
        markWindow.request(force === true);
    }

    function armQuickActions() {
        if (root.quickActionKind === "" || root.count === 0 || root.currentIndex < 0)
            return;
        var item = root.access ? root.access.get(root.currentIndex) : null;
        if (!item)
            return;

        var actions = null;
        if (root.hasSelection) {
            actions = quickVerbs.forSelection(root.quickActionKind, root.selectionIds(root.quickActionField), root.selectionStarred(root.quickActionField));
        } else {
            var preview = function (level) {
                if (root.access && root.access.setProperty && item.rating !== undefined)
                    root.access.setProperty(root.currentIndex, "rating", level);
            };
            actions = quickVerbs.forItem(root.quickActionKind, item, Backend.cachedAlbumIds.indexOf(item.albumId) !== -1, preview);
        }
        if (!actions)
            return;

        quickVerbs.wrapFavorite(actions, function (on) {
            rowState.markStarred(on);
        });

        var row = root.view ? root.view.itemAtIndex(root.currentIndex) : null;
        if (!row)
            row = root.view;
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
        model: root.access
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

    // ── View ─────────────────────────────────────────────────────────────

    ScrollMemory {
        id: scroll
    }

    /// The single animation: one short OutCubic glide from the live position
    /// to the new target on each cursor move, matching the pre-refactor feel.
    NumberAnimation {
        id: glide
        target: root.view
        property: "contentY"
        duration: root.followMs
        easing.type: Easing.OutCubic
        onFinished: {
            if (root.debugFollow && root.view)
                console.log("ScrollContent follow residual",
                    Math.abs(root.targetY(root.currentIndex) - root.view.contentY));
            root._oneShotMs = 0;
        }
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
            reuseItems: root.reuseRows && !root.animateDisplacement
            delegate: root.rowDelegate
            spacing: root.rowSpacing
            header: Item {
                width: list.width
                height: Math.round(headerLoader.height + root._shortContentGap)
                Loader {
                    id: headerLoader
                    width: parent.width
                    sourceComponent: root.headerContent
                }
            }
            boundsBehavior: Flickable.StopAtBounds
            highlightFollowsCurrentItem: false
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

    property int _oneShotMs: 0

    // ── Derived-geometry triggers ────────────────────────────────────────

    onHeightChanged: {
        root.captureRowBase();
        root.sync(false);
    }
    onRowRefHeightChanged: {
        root.captureRowBase();
        root.sync(false);
    }
    onRowRatioChanged: root.sync(false)
    onRowSpacingChanged: root.sync(false)
    onRowStrideChanged: root.sync(false)
    onAdaptiveRowHeightChanged: root.sync(false)
    onMinYChanged: root.sync(false)
    onMaxYChanged: root.sync(false)
    onHeaderHeightChanged: root.sync(false)
    onFooterHeightChanged: root.sync(false)
    onVisibleChanged: {
        if (root.visible)
            root.sync(false);
    }

    Connections {
        target: root.view
        function onMovementStarted() {
            root.stopGlide();
        }
        function onContentYChanged() {
            root.updateVisibleBounds();
        }
        function onCountChanged() {
            if (target.count === 0) {
                if (root.currentIndex !== -1)
                    root.currentIndex = -1;
            } else if (root.currentIndex >= target.count) {
                root.currentIndex = target.count - 1;
            } else if (root.currentIndex < 0 && root.followCursor) {
                root.currentIndex = 0;
            }
            root.sync(false);
        }
    }
}
