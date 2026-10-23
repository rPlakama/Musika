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
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Effects
import musika
import "Theme.js" as Theme

// Library shell: status bar on top, the scoped library below it, player bar at the
// bottom.
//
// Focus model (vi-like): `activeRegion` is the cursor and its border stays lit
// at all times. Cross-region reach is `/` (ARC-UI-2): a search jump moves the
// cursor between the main list and the status region, so the old Ctrl-W
// region picker is retired.
Item {
    id: root

    // 1 = main, 2 = status (top). The player bar is decoration driven by
    // global binds (UI-51), never a keyboard destination.
    property int activeRegion: 1
    property bool nowPlayingExpanded: false
    property int currentSegment: 0
    // A scope switch starts the target segment at row 0 instead of
    // restoring/settling toward its previous cursor.
    property bool _resetSegmentOnShow: false

    // ARC-45: bounded travel log, one row per visited place (2-D):
    // [segment, fullscreen, pageCount, page×8...]. A page block is
    // [kind, id, title/name, artist, meta, cover, starred, ""].
    property var placeHistory: []
    property int placeIndex: -1
    property bool _placing: false
    readonly property int placeMax: 32

    onCurrentSegmentChanged: root.recordPlace()
    onNowPlayingExpandedChanged: root.recordPlace()

    readonly property real barH: height * Theme.topBarRatio
    readonly property real playH: height * Theme.playerRatio
    readonly property real bodyH: height - barH - playH

    // View transitions (motion §3.1): fade in with decelerate, fade out with
    // accelerate. One factory reused by push/pop/replace.
    component EnterView: Transition {
        NumberAnimation {
            property: "opacity"; from: 0; to: 1
            duration: Theme.motion.standard
            easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.decelerate
        }
    }

    component ExitView: Transition {
        NumberAnimation {
            property: "opacity"; from: 1; to: 0
            duration: Theme.motion.quick
            easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.accelerate
        }
    }

    function showSegment(index) {
        // ARC-45: a segment switch pops every page; record the final place
        // once, not each intermediate pop.
        var restoring = _placing;
        _placing = true;
        if (currentSegment !== index)
            root._resetSegmentOnShow = true;
        currentSegment = index;
        while (contentStack.depth > 1)
            contentStack.pop();
        // A scope switch starts the target segment at row 0; a page pop
        // still lands on the cursor it left behind.
        activeRegion = 1;
        _placing = restoring;
        recordPlace();
    }

    /// A segment becoming visible either lands where it was (page return) or
    /// starts at row 0 (scope switch). The reset also clears the segment.s
    /// saved offset, so the next payload cannot restore the old position.
    function _landOrResetSegment(v) {
        if (!v)
            return;
        if (root._resetSegmentOnShow) {
            root._resetSegmentOnShow = false;
            if (v.count > 0 && v.resetViewNow)
                v.resetViewNow();
            else if (v.resetView)
                v.resetView();
            return;
        }
        if (v.landCursor)
            v.landCursor();
    }

    // ARC-45: the travel log. Every place-changing navigation appends a row
    // (segment/fullscreen/depth notify) unless this guard says a restore is
    // rebuilding the stack; adjacent duplicates collapse and a fresh move
    // after `[b` drops the forward rows, browser-style.
    function snapshotPlace() {
        var pages = [];
        for (var i = 1; i < contentStack.depth; ++i) {
            var page = contentStack.get(i);
            if (!page)
                continue;
            if (page.albumId !== undefined) {
                pages.push("album", page.albumId, page.albumTitle,
                    page.albumArtist, page.albumMeta, page.albumCover,
                    page.albumStarred === true ? 1 : 0, "");
            } else if (page.artistId !== undefined) {
                pages.push("artist", page.artistId, page.artistName, "",
                    "", page.artistCover, 0, "");
            }
        }
        var row = [root.currentSegment,
            root.nowPlayingExpanded ? 1 : 0, pages.length / 8];
        for (var p = 0; p < pages.length; ++p)
            row.push(pages[p]);
        return row;
    }

    function recordPlace() {
        if (_placing || !contentStack || !placeHistory)
            return;
        var row = snapshotPlace();
        var key = JSON.stringify(row);
        if (placeIndex >= 0 && placeIndex < placeHistory.length) {
            if (JSON.stringify(placeHistory[placeIndex]) === key)
                return;
            if (placeIndex < placeHistory.length - 1)
                placeHistory = placeHistory.slice(0, placeIndex + 1);
        }
        var rows = placeHistory.slice();
        rows.push(row);
        while (rows.length > placeMax)
            rows.shift();
        placeHistory = rows;
        placeIndex = rows.length - 1;
    }

    /// Transient overlays are not places: the first history press closes
    /// them and the next one moves.
    function closeTransientOverlays() {
        var closed = false;
        if (lineSearch.opened) {
            lineSearch.close();
            closed = true;
        }
        if (searchOverlay.opened) {
            searchOverlay.close();
            closed = true;
        }
        if (quickActions.armed) {
            quickActions.cancel();
            closed = true;
        }
        return closed;
    }

    function movePlace(delta) {
        if (_placing)
            return;
        if (closeTransientOverlays())
            return;
        var target = placeIndex + delta;
        if (target < 0 || target >= placeHistory.length)
            return;
        placeIndex = target;
        applyPlace(placeHistory[target]);
    }

    function applyPlace(row) {
        if (!row || row.length < 3)
            return;
        var pageCount = Number(row[2]) || 0;
        _placing = true;
        // One cut, not a cascade of push/pop animations.
        if (contentStack.depth > 1)
            contentStack.pop(null, StackView.Immediate);
        currentSegment = Number(row[0]) || 0;
        activeRegion = 1;
        var at = 3;
        for (var i = 0; i < pageCount; ++i) {
            if (at + 7 >= row.length)
                break;
            var kind = row[at];
            var id = row[at + 1];
            var title = row[at + 2];
            var artist = row[at + 3];
            var meta = row[at + 4];
            var cover = row[at + 5];
            var starred = row[at + 6] === 1;
            at += 8;
            if (kind === "album") {
                Backend.rememberAlbum(id);
                contentStack.push(albumEntryComponent, {
                    albumId: id,
                    albumTitle: title,
                    albumArtist: artist,
                    albumMeta: meta,
                    albumCover: cover,
                    albumStarred: starred
                }, StackView.Immediate);
            } else if (kind === "artist") {
                contentStack.push(artistEntryComponent, {
                    artistId: id,
                    artistName: title,
                    artistCover: cover
                }, StackView.Immediate);
            }
        }
        nowPlayingExpanded = row[1] === 1;
        _placing = false;
    }

    // Global fullscreen toggle (dispatched by the app-level handler before
    // screen/region routing, so it works from any region and overlay state).
    function toggleFullscreen() {
        nowPlayingExpanded = !nowPlayingExpanded;
    }

    // Clipboard paste (VI-21): the text lands visibly in the search field so
    // the user can correct it; Enter plays the selection.
    function pasteSearch(text) {
        clearMainSelection();
        searchOverlay.openWith(text);
    }

    // VI-4: entering another surface (search) ends the main list.s
    // visual selection.
    function clearMainSelection() {
        var view = contentStack.currentItem;
        if (view && view.clearSelection)
            view.clearSelection();
    }

    // P-8: returning from an artist entry always lands on Main — the plain
    // album list, unfiltered and out of any other segment or search.
    function leaveArtistEntry() {
        showSegment(0);
    }

    // Single album-page push used by the album list, the artist entry and the
    // search overlay, so every surface lands in the same page with the same
    // header payload (`starred` feeds the ARC-I-2 header heart).
    function openAlbum(albumId, title, artist, year, tracks, cover, starred) {
        // P-6 S2: the last opened album page is what a restart reopens.
        Backend.rememberAlbum(albumId);
        var meta = [];
        if (year.length > 0)
            meta.push(year);
        if (tracks.length > 0)
            meta.push(tracks + " tracks");
        contentStack.push(albumEntryComponent, {
            albumId: albumId,
            albumTitle: title,
            albumArtist: artist,
            albumCover: cover,
            albumMeta: meta.join("   ·   "),
            albumStarred: starred === true
        });
    }

    // ARC-43/ARC-UI-1/ARC-I: a surface arms the Alt-hold overlay through here;
    // the action closures come from the shared QuickActions verb sets. The
    // search arms its own active column while open; the command line blocks.
    function armQuickActions(target, actions) {
        if (lineSearch.opened)
            return;
        quickActions.arm(target, actions);
    }

    // ARC-43/ARC-UI-1/ARC-I: Alt pressed — the active surface anchors the
    // overlay on its keyboard cursor row/card through its own
    // `armQuickActions`: the search's active column while it is open, the
    // fullscreen queue when the player is up, the current view otherwise.
    function beginQuickActions() {
        // A sticky menu (the wipe chooser) keeps the overlay: Alt must not
        // replace its config with the current view's verbs.
        if (quickActions.armed && quickActions.sticky)
            return;
        // ARC-UI-2: the command line blocks arming too.
        if (lineSearch.opened)
            return;
        if (searchOverlay.opened) {
            searchOverlay.armQuickActions();
            return;
        }
        // Reusable gate: any surface opts out of the Alt menu with
        // `altActionsAvailable: false` (the status bar's sync scope does).
        var view = root.activeRegion === 2
            ? topBar
            : (root.nowPlayingExpanded ? nowPlaying : contentStack.currentItem);
        if (view && view.armQuickActions && view.altActionsAvailable !== false)
            view.armQuickActions();
    }

    /// Destructive maintenance menu (the SuperiorBar trashcan — a click, Alt
    /// while it is scoped, or the `wipe_databases` IPC command). The Alt path
    /// is not sticky: releasing over a quadrant fires it, releasing over the
    /// neutral centre cancels. Click/IPC opens are sticky (no Alt release to
    /// fire them) and cancel by clicking outside or Enter on neutral.
    function armWipeMenu(target, sticky) {
        if (Backend.librarySyncing) {
            toast.show("sync", "Wait for the sync to finish before wiping.");
            return;
        }
        if (target)
            quickActions.arm(target, verbs.wipe(sticky));
    }

    QuickActions {
        id: verbs
    }

    // ARC-UI-2: `/` treats the whole window as the buffer. In the plain
    // context every visible row surface answers at once — status region and
    // the main list — in window order; when a modal owns the
    // context (the FTS overlay, fullscreen) only it answers, as its routing
    // owns Enter. Each entry is a tiny proxy so a jump can also move the
    // region cursor that the following context-Enter (VI-16) routes to.
    function lineSearchTargets() {
        function wrap(surface, focus) {
            if (!surface || !surface.searchRows)
                return null;
            return {
                searchRows: function () { return surface.searchRows(); },
                searchJump: function (row) {
                    focus();
                    surface.searchJump(row);
                },
                setQuery: function (q, row) {
                    try {
                        surface.searchQuery = q;
                        surface.searchCurrentRow = row;
                    } catch (e) {
                        // Surfaces without row text (none today) ignore it.
                    }
                }
            };
        }
        function add(list, surface, focus) {
            var entry = wrap(surface, focus);
            if (entry)
                list.push(entry);
        }
        function noFocus() {}
        var out = [];
        if (searchOverlay.opened) {
            add(out, searchOverlay, noFocus);
            return out;
        }
        if (nowPlayingExpanded && nowPlaying) {
            add(out, nowPlaying, noFocus);
            return out;
        }
        add(out, topBar, function () { root.activeRegion = 2; });
        add(out, contentStack.currentItem, function () { root.activeRegion = 1; });
        return out;
    }

    // Tab's small-action walk: the main list's Shuffle chip, then the
    // status bar's Sync → Wipe scope, then back to the list — the controls
    // stay reachable without leaving region 1 for the top bar. Shift+Tab
    // walks the same circle backwards (list → wipe → sync → chip → list).
    // Surfaces with their own Tab stops (ArtistEntry's chips, the fullscreen
    // player's panes) keep them.
    function tabWalk(action) {
        var back = action === "pane_cycle_back";
        // The stack's head is the segment host; the live list is its
        // `activeView` (a pushed page has none and is the view itself).
        var host = contentStack.currentItem;
        var view = host && host.activeView !== undefined ? host.activeView : host;
        var listy = view && view.hasHeaderChip !== undefined;

        if (activeRegion === 1 && !listy) {
            // Surfaces with their own Tab stops keep them (ArtistEntry's
            // chips, AlbumEntry's cover/back cycle); a plain page steps
            // straight into the status bar.
            if (view && (view.chipScope !== undefined || view.tabStops === true)) {
                host.handleAction(action);
                return;
            }
            activeRegion = 2;
            topBar.scope = back ? 1 : 0;
            return;
        }

        if (activeRegion === 1) {
            var chipped = view.hasHeaderChip === true;
            var onChip = chipped && view.headerScope === 0;
            if (back) {
                if (onChip) {
                    view.handleAction("pane_cycle");      // chip → list
                    return;
                }
                activeRegion = 2;                         // list → wipe
                topBar.scope = 1;
                return;
            }
            if (chipped && !onChip) {
                view.handleAction("pane_cycle");          // list → chip
                return;
            }
            if (onChip)
                view.handleAction("pane_cycle");          // chip → list
            activeRegion = 2;                             // …then sync
            topBar.scope = 0;
            return;
        }

        if (activeRegion === 2) {
            if (back ? topBar.scope === 0 : topBar.scope === 1) {
                activeRegion = 1;                         // …back to the list
                if (back && view && view.hasHeaderChip === true)
                    view.handleAction("pane_cycle");      // …and ring the chip
                return;
            }
            topBar.scope = back ? 0 : 1;                  // sync ↔ wipe
            return;
        }

    }

    // Actions dispatched from the global key handler, scoped to the active
    // region: movement only ever affects the region the cursor is in.
    function handleAction(action) {
        // Command pump: the search overlay (and external IPC via `ipc.sock`)
        // both express this as `play_song_in_album <song-id>`.
        if (action.indexOf("play_song_in_album ") === 0) {
            Backend.playSongInAlbum(action.substring("play_song_in_album ".length));
            return;
        }

        // Command pump: `download_song <song-id>` marks and fetches one track
        // (CACHE-8). Any future UI trigger goes through the same slot.
        if (action.indexOf("download_song ") === 0) {
            Backend.downloadSong(action.substring("download_song ".length));
            return;
        }

        // ARC-45: previous/next place is global like Back. Overlays are not
        // places, so the first press closes them and the second moves.
        if (action === "prev_place" || action === "next_place") {
            root.movePlace(action === "prev_place" ? -1 : 1);
            return;
        }

        // ARC-43: keyboard aiming while Alt is held (h/j/k/l) and Esc cancel;
        // sent by the window's key handler, never by the keymap.
        if (action.indexOf("quick_aim ") === 0) {
            quickActions.aim(action.substring("quick_aim ".length));
            return;
        }
        if (action === "quick_cancel") {
            quickActions.dismiss();
            return;
        }
        // UI-60: Shift-J/K over the aimed favourite slot steps its rating.
        if (action.indexOf("quick_rating ") === 0) {
            quickActions.adjustRating(
                action.substring("quick_rating ".length) === "up" ? 1 : -1);
            return;
        }

        // Sticky quadrant menu (the wipe chooser): h/j/k/l or arrows aim,
        // Enter fires the aimed wipe (at neutral it just closes). Esc is not
        // a cancel here either — a click outside the board closes it.
        if (quickActions.armed && quickActions.sticky) {
            if (action === "move_up")
                quickActions.aim("north");
            else if (action === "move_down")
                quickActions.aim("south");
            else if (action === "move_left")
                quickActions.aim("west");
            else if (action === "move_right")
                quickActions.aim("east");
            else if (action === "enter")
                quickActions.confirm();
            return;
        }

        // IPC command parity: `wipe_databases` opens the wipe menu anchored
        // on the trashcan (the keyboard path is Alt while it is scoped).
        if (action === "wipe_databases") {
            root.armWipeMenu(topBar.wipeAnchor, true);
            return;
        }

        // ARC-UI-2 M0: the `/` command line is the topmost modal while it is
        // up (the keymap is dead in Backend, so this normally only sees IPC);
        // `/` opens it over whatever surface or overlay is on top.
        if (lineSearch.opened) {
            lineSearch.handleAction(action);
            return;
        }
        if (action === "search_action") {
            clearMainSelection();
            lineSearch.open(root.lineSearchTargets());
            return;
        }
        // The floating search is modal: while open, all keys go to it.
        if (searchOverlay.opened) {
            searchOverlay.handleAction(action);
            return;
        }
        // Scope switching (Shift-J/K) walks the unified library types.
        if (action === "scope_next" || action === "scope_prev") {
            if (nowPlayingExpanded)
                return;
            var scopeCount = 5;
            var nextScope = (currentSegment
                + (action === "scope_next" ? 1 : -1) + scopeCount) % scopeCount;
            root.showSegment(nextScope);
            return;
        }
        // Transport is global once the modals above have had their say:
        // the search overlay owns its input, so Space cannot leak through
        // (VI-18).
        if (action === "play_pause") {
            Backend.playerToggle();
            return;
        }
        // List ordering (Shift-L / Shift-H) is scoped to the visible list:
        // Songs cycles the song order, Artists its own Name/Albums cycle,
        // everything else the album order (one shared cycle per scope).
        if (action === "order_next" || action === "order_prev") {
            var forward = action === "order_next";
            if (currentSegment === 1) {
                if (forward)
                    Backend.songOrderNext();
                else
                    Backend.songOrderPrev();
            } else if (currentSegment === 2) {
                if (forward)
                    Backend.artistOrderNext();
                else
                    Backend.artistOrderPrev();
            } else {
                if (forward)
                    Backend.albumOrderNext();
                else
                    Backend.albumOrderPrev();
            }
            return;
        }
        // Ctrl-Q toggles the queue drawer from anywhere: enter the player if
        // needed, then slide the queue in or out.
        if (action === "queue_toggle") {
            if (!nowPlayingExpanded) {
                nowPlayingExpanded = true;
                if (nowPlaying)
                    nowPlaying.openQueue();
                else
                    nowPlayingLoader.openQueue = true;
            } else if (nowPlaying) {
                nowPlaying.toggleQueue();
            }
            return;
        }

        // VI-4: `v`/`V` start visual row selection on the focused list — the
        // fullscreen queue when the player is up. The search overlay handles
        // its own (modal routing above).
        if (action === "visual" || action === "visual_all") {
            if (nowPlayingExpanded) {
                if (nowPlaying && nowPlaying.queueOpen) {
                    if (action === "visual")
                        nowPlaying.toggleSelection();
                    else
                        nowPlaying.selectAll();
                }
                return;
            }
            if (activeRegion !== 1)
                return;
            var listView = contentStack.currentItem;
            if (!listView)
                return;
            if (action === "visual" && listView.toggleSelection)
                listView.toggleSelection();
            else if (action === "visual_all" && listView.selectAll)
                listView.selectAll();
            return;
        }

        // Search is global scope: open the floating search from any region.
        if (action === "search") {
            clearMainSelection();
            searchOverlay.open();
            return;
        }
        // `i` starts typing: opens the floating search in INSERT mode from any
        // region (VI-19); inside the overlay the modal routing above re-enters
        // typing instead.
        if (action === "insert_mode") {
            clearMainSelection();
            searchOverlay.openWith("");
            return;
        }
        // Transport is global (VI-10/Q-9/UI-51): j/k keeps the list cursor,
        // Ctrl-N/Ctrl-P (and n/p) skip tracks and ,/. seek ±5 s wherever a
        // list owns focus. The player bar is decoration and owns no keys.
        if (action === "next_track") {
            Backend.playerNext();
            return;
        }
        if (action === "prev_track") {
            Backend.playerPrevious();
            return;
        }
        if (action === "seek_back") {
            Backend.playerSeek(Math.max(0, Backend.position - 5 * Math.max(1, Backend.keyCount)));
            return;
        }
        if (action === "seek_forward") {
            Backend.playerSeek(Backend.position + 5 * Math.max(1, Backend.keyCount));
            return;
        }
        // Library sync is global too (`r`): the sync button stays clickable,
        // but the keyboard never has to reach the status bar to refresh.
        if (action === "refresh") {
            Backend.syncLibrary();
            return;
        }

        if (action === "back") {
            // Fullscreen is the topmost context: Esc first closes the queue
            // drawer, then ends a queue selection, then leaves the player.
            if (nowPlayingExpanded) {
                if (nowPlaying && nowPlaying.queueOpen) {
                    nowPlaying.closeQueue();
                    return;
                }
                if (nowPlaying && nowPlaying.hasSelection) {
                    nowPlaying.clearSelection();
                    return;
                }
                nowPlayingExpanded = false;
                return;
            }
            // VI-4: the first Esc ends the focused list's visual selection.
            if (contentStack.currentItem && contentStack.currentItem.hasSelection) {
                contentStack.currentItem.clearSelection();
                return;
            }
            // P-8: an artist entry is a detour — back lands on Main.
            if (contentStack.currentItem && contentStack.currentItem.exitsToMain)
                root.leaveArtistEntry();
            else if (contentStack.depth > 1)
                contentStack.pop();
            return;
        }

        // Fullscreen is modal for navigation: pane and queue keys belong to
        // the player view while it is open (Esc, Space and ,/. stay global).
        if (nowPlayingExpanded) {
            if (nowPlaying && nowPlaying.handleAction)
                nowPlaying.handleAction(action);
            return;
        }

        // Tab walks the shell's small actions across surfaces (Shuffle →
        // Sync → Wipe).
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            tabWalk(action);
            return;
        }

        // The status bar is a real surface (VI-17): Enter fires the scoped
        // control, h/l cycle the scope (sync ↔ wipe — Tab's assignment with
        // the hands still on hjkl), down returns to the main list.
        if (activeRegion === 2) {
            if (action === "move_down") {
                activeRegion = 1;
                return;
            }
            if (action === "move_left" || action === "move_right") {
                topBar.handleAction("pane_cycle");
                return;
            }
        }

        // Up at the very top of a surface hands the cursor to the status
        // region: nothing above the first row (or the album toolbar) means
        // the up motion reaches further up.
        if (action === "move_up") {
            if (activeRegion === 1) {
                var top = contentStack.currentItem;
                if (top && top.onToolbar !== undefined && top.onToolbar) {
                    // AlbumEntry: row 0's `k` lands on the toolbar first; only
                    // after that is the up edge the status region.
                    activeRegion = 2;
                    return;
                }
                if (top && top.onToolbar === undefined && top.currentIndex !== undefined
                        && top.currentIndex <= 0) {
                    activeRegion = 2;
                    return;
                }
            }
        }

        switch (activeRegion) {
        case 1:
            if (contentStack.currentItem && contentStack.currentItem.handleAction)
                contentStack.currentItem.handleAction(action);
            break;
        case 2:
            topBar.handleAction(action);
            break;
        }
    }

    ColumnLayout {
        id: shell
        anchors.fill: parent
        spacing: 0

        SuperiorBar {
            id: topBar
            Layout.fillWidth: true
            Layout.preferredHeight: root.barH
            active: root.activeRegion === 2
            onWipeRequested: (target, sticky) => root.armWipeMenu(target, sticky)
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            ColumnLayout {
                id: mainArea
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                StackView {
                    id: contentStack
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    initialItem: segmentHost

                    pushEnter: EnterView {}
                    pushExit: ExitView {}
                    popEnter: EnterView {}
                    popExit: ExitView {}
                    replaceEnter: EnterView {}
                    replaceExit: ExitView {}

                    // ARC-45: every page push/pop is a place change.
                    onDepthChanged: root.recordPlace()

                    // The five segments stay alive once instantiated —
                    // switching is visibility, not a rebuild, so returning to
                    // a tab lands instantly with its cursor, scroll memory and
                    // model intact (no skeleton beat, no re-restore). Pages
                    // (album/artist) still push/pop above this host, and every
                    // row-surface call Home makes on `currentItem` forwards to
                    // the visible view.
                    Item {
                        id: segmentHost

                        // SCROLLMODELANALYSIS §3: a returning surface lands
                        // its cursor instantly — a page pop must not walk a
                        // far cursor back from wherever the view sat.
                        onVisibleChanged: {
                            if (visible)
                                root._landOrResetSegment(activeView);
                        }

                        property int _viewRevision: 0

                        // The `count` read is the dependency that re-evaluates
                        // this when the Repeater builds its delegates: a bare
                        // `itemAt()` lookup before they exist returns null and
                        // registers nothing, so the binding would stay null
                        // forever (the "scroll unavailable" bug).
                        readonly property Item activeView: {
                            _viewRevision;
                            if (segmentLoaders.count === 0)
                                return null;
                            var loader = segmentLoaders.itemAt(root.currentSegment);
                            return loader ? loader.item : null;
                        }
                        readonly property bool hasSelection: activeView
                            ? activeView.hasSelection === true : false
                        readonly property bool exitsToMain: false
                        readonly property int currentIndex: (activeView
                            && activeView.currentIndex !== undefined)
                            ? activeView.currentIndex : -1

                        /// ARC-UI-2: the live `/` session forwards to the
                        /// visible view (see `Home.lineSearchTargets`).
                        property string searchQuery: ""
                        property int searchCurrentRow: -1
                        onSearchQueryChanged: _applySearch()
                        onSearchCurrentRowChanged: _applySearch()

                        function _applySearch() {
                            var v = activeView;
                            if (!v)
                                return;
                            try {
                                v.searchQuery = searchQuery;
                                v.searchCurrentRow = searchCurrentRow;
                            } catch (e) {
                            }
                        }

                        function handleAction(action) {
                            var v = activeView;
                            if (v && v.handleAction)
                                v.handleAction(action);
                        }
                        function clearSelection() {
                            var v = activeView;
                            if (v && v.clearSelection)
                                v.clearSelection();
                        }
                        function toggleSelection() {
                            var v = activeView;
                            if (v && v.toggleSelection)
                                v.toggleSelection();
                        }
                        function selectAll() {
                            var v = activeView;
                            if (v && v.selectAll)
                                v.selectAll();
                        }
                        function moveWithinGrid(action) {
                            var v = activeView;
                            return v && v.moveWithinGrid ? v.moveWithinGrid(action) : false;
                        }
                        function armQuickActions() {
                            var v = activeView;
                            if (v && v.armQuickActions)
                                v.armQuickActions();
                        }
                        function searchRows() {
                            return activeView && activeView.searchRows ? activeView.searchRows() : [];
                        }
                        function searchJump(row) {
                            var v = activeView;
                            if (v && v.searchJump)
                                v.searchJump(row);
                        }

                        Repeater {
                            id: segmentLoaders
                            model: 5

                            delegate: Loader {
                                required property int index
                                anchors.fill: parent
                                active: index === root.currentSegment
                                visible: index === root.currentSegment
                                onItemChanged: {
                                    if (index === root.currentSegment)
                                        segmentHost._viewRevision++;
                                }
                                // SCROLLMODELANALYSIS §3: a segment coming
                                // back lands its cursor instantly.
                                onVisibleChanged: {
                                    if (visible)
                                        root._landOrResetSegment(item);
                                }
                                onLoaded: {
                                    if (visible)
                                        root._landOrResetSegment(item);
                                }
                                sourceComponent: index === 1 ? songListComponent
                                    : index === 2 ? artistListComponent
                                    : index === 3 ? playlistListComponent
                                    : index === 4 ? radioListComponent
                                    : albumListComponent
                            }
                        }
                    }

                    Component {
                        id: albumListComponent
                        AlbumList {
                            onAlbumActivated: (albumId, title, artist, year, tracks, cover, starred) =>
                                root.openAlbum(albumId, title, artist, year, tracks, cover, starred)
                            onQuickAction: (target, actions) =>
                                root.armQuickActions(target, actions)
                        }
                    }

                    Component {
                        id: songListComponent
                        SongList {
                            onQuickAction: (target, actions) =>
                                root.armQuickActions(target, actions)
                        }
                    }

                    Component {
                        id: artistListComponent
                        ArtistList {
                            onArtistActivated: (artistId, artistName, artistCover) => {
                                root.nowPlayingExpanded = false;
                                contentStack.push(artistEntryComponent, {
                                    artistId: artistId,
                                    artistName: artistName,
                                    artistCover: artistCover
                                });
                            }
                            // UI-60: the artist row arms the same Alt overlay
                            // as albums/songs — without this relay the south
                            // rating step never fired, so the Artists list's
                            // favourite rectangle never reloaded.
                            onQuickAction: (target, actions) =>
                                root.armQuickActions(target, actions)
                        }
                    }

                    Component {
                        id: artistEntryComponent
                        ArtistEntry {
                            onBack: root.leaveArtistEntry()
                            onAlbumActivated: (albumId, title, artist, year, tracks, cover, starred) =>
                                root.openAlbum(albumId, title, artist, year, tracks, cover, starred)
                            onQuickAction: (target, actions) =>
                                root.armQuickActions(target, actions)
                        }
                    }

                    Component {
                        id: playlistListComponent
                        PlaylistList {
                            // ARC-44 M0: the list lands first; the entry page
                            // (M2) hangs off this signal.
                        }
                    }

                    Component {
                        id: radioListComponent
                        PlaceholderList {
                            title: "Radio"
                            message: "Internet radio stations are not indexed yet."
                        }
                    }

                    Component {
                        id: albumEntryComponent
                        AlbumEntry {
                            onBack: contentStack.pop()
                            // ARC-I-2: the page arms the shared Alt overlay on
                            // its song rows.
                            onQuickAction: (target, actions) =>
                                root.armQuickActions(target, actions)
                        }
                    }
                }
            }
        }

        InferiorBar {
            id: player
            Layout.fillWidth: true
            Layout.preferredHeight: root.playH
        }
    }

    Loader {
        id: nowPlayingLoader
        anchors.fill: parent
        active: root.nowPlayingExpanded || opacity > 0
        opacity: root.nowPlayingExpanded ? 1 : 0
        visible: opacity > 0
        // Ctrl-Q asks a freshly created player to open the queue drawer.
        property bool openQueue: false

        sourceComponent: Component {
            NowPlaying {
                anchors.fill: parent
                onExitRequested: root.nowPlayingExpanded = false
                onQuickAction: (target, actions) => root.armQuickActions(target, actions)
            }
        }

        onLoaded: {
            if (openQueue) {
                openQueue = false;
                if (item)
                    item.openQueue();
            }
        }

        // Enter/leave is a scene change, not a highlight flip: the black
        // backdrop fades in/out (`emphasized`), decelerating in and
        // accelerating out (motion §3.1). Toggling mid-fade retargets the same
        // animation, so repeated enter/leave stays one smooth loop.
        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.emphasized
                easing.type: Easing.Bezier
                easing.bezierCurve: root.nowPlayingExpanded ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }
    }

    readonly property Item nowPlaying: nowPlayingLoader.item

    // ARC-43: the Alt-hold quick-action overlay. Alt anchors it on the active
    // view's cursor item through `armQuickActions`; h/j/k/l aim and the Alt
    // release fires. Acrylic tiles over the shell, below the toast so errors
    // stay readable mid-gesture.
    AltActionsOverlay {
        id: quickActions
        anchors.fill: parent
        // ARC-I: while the search is open its column arms the same overlay,
        // which must paint above the search panel (search z = zOverlay).
        z: searchOverlay.opened ? Theme.zOverlay + 1 : Theme.zRegion + 1
        backdrop: (root.nowPlayingExpanded && nowPlaying) ? nowPlaying
            : (searchOverlay.opened ? searchOverlay : shell)
        // UI-60 "bright": the rating step's stars outlive the tile collapse.
        onRatingStepped: (x, y) => ratingBurst.burstAt(x, y)
    }

    RatingBurst {
        id: ratingBurst
        anchors.fill: parent
        z: quickActions.z + 1
    }

    Toast {
        id: toast
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: root.playH + root.height * 0.02
        // The banner grows with its message; the cap is everything above the
        // player bar, so a long error can take the whole screen.
        maxHeight: root.height - root.playH - root.height * 0.06
        z: Theme.zToast
        // UI-55: blur whatever the toast actually floats over.
        backdrop: (root.nowPlayingExpanded && nowPlaying) ? nowPlaying : shell
        onRetry: {
            if (toast.kind === "sync")
                Backend.syncLibrary();
            else
                Backend.retryPlayback();
        }
    }

    // VI-7: the count register's chip — the number being typed (`3` before
    // `j`). Fades in over `micro`; the clear is instant (the visibility
    // drops with the register, no fade-out).
    Rectangle {
        id: countChip
        anchors.right: parent.right
        anchors.rightMargin: root.width * 0.02
        anchors.bottom: parent.bottom
        anchors.bottomMargin: root.playH + root.height * 0.02
        width: Math.max(height, countLabel.implicitWidth + height * 0.6)
        height: root.height * 0.045
        radius: Theme.radius
        color: Palette.surfaceContainerHigh
        visible: Backend.keyCount > 0
        opacity: Backend.keyCount > 0 ? 1 : 0
        z: Theme.zToast

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.micro
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }

        FadeText {
            id: countLabel
            anchors.centerIn: parent
            text: Backend.keyCount > 0 ? String(Backend.keyCount) : ""
            color: Palette.onSurface
            font.family: Theme.fonts.body
            font.pixelSize: root.height * 0.026
            animated: false
        }
    }

    SearchOverlay {
        id: searchOverlay
        anchors.fill: parent
        // UI-55: blur the shell, or the fullscreen player when it is up.
        backdrop: (root.nowPlayingExpanded && nowPlaying) ? nowPlaying : shell

        // ARC-I: the active column arms the shared Alt overlay.
        onQuickAction: (target, actions) => root.armQuickActions(target, actions)

        // A search song hit goes through the same command pump as external
        // IPC, so the flow stays scriptable and single-sourced.
        onSongActivated: (id) => Backend.handleCommand("play_song_in_album " + id)
        onAlbumActivated: (albumId, title, artist, year, tracks, cover, starred) =>
            root.openAlbum(albumId, title, artist, year, tracks, cover, starred)
        onArtistActivated: (artistId, artistName, artistCover) => {
            root.nowPlayingExpanded = false;
            contentStack.push(artistEntryComponent, {
                artistId: artistId,
                artistName: artistName,
                artistCover: artistCover
            });
        }
    }

    // ARC-UI-2 M0: the `/` in-surface search command line. `zCommand` rides
    // over every overlay; the flat bar sits at the very bottom over its own
    // 50% black scrim.
    LineSearch {
        id: lineSearch
        anchors.fill: parent
        // The commit closes the prompt, then the matched row's own
        // context-Enter runs through the normal action router (VI-16).
        onEntered: root.handleAction("enter")
    }

    Connections {
        target: Backend
        function onErrorRaised(kind, message) {
            toast.show(kind, message);
        }
        // ARC-43: holding Alt opens the quick-action overlay on the active
        // view's keyboard cursor item.
        function onAlt_held_changed() {
            if (Backend.altHeld)
                root.beginQuickActions();
        }
        // Q-10 (roadmap D-4): activating a song opens Fullscreen Playback.
        // Album/artist activation keeps its navigation; the album-page IPC
        // flow below clears the flag again when it pushes the album page.
        function onPlaybackStarted() {
            nowPlayingExpanded = true;
        }
        // P-6 S2: restored session. The queue loads paused (no autoplay, no
        // fullscreen) and the remembered album page reopens; both are pumped
        // so the backend is not re-entered while the signal is emitted.
        function onStatusRestored(payload) {
            var data = JSON.parse(payload);
            Qt.callLater(function() {
                if (data.queue && data.queue.length > 0)
                    Backend.playerLoadPaused(data.queue, data.start);
                if (data.album)
                    root.openAlbum(data.album.id, data.album.title,
                                   data.album.artist ? data.album.artist : "",
                                   data.album.year ? String(data.album.year) : "",
                                   String(data.album.tracks),
                                   data.album.cover ? data.album.cover : "",
                                   data.album.starred === true);
                // Session return: the restored album's rows must render
                // their state marks on arrival — the page only installs its
                // mark window when its payload lands, and without this the
                // marks waited for the first cursor move (a row
                // interaction). One tick later the window is up; the shared
                // reload re-reads it and notifies every mark.
                Qt.callLater(function() { Backend.TriggerRowReload(); });
            });
        }
        // Single pumped event from `playSongInAlbum`: the queue + album
        // context. Deferred so the backend is not re-entered while the signal
        // is still being emitted.
        function onDataReady(channel, payload) {
            if (channel === "song_album") {
                var data = JSON.parse(payload);
                Qt.callLater(function() {
                    Backend.playerLoad(data.queue, data.start);
                    if (!data.album)
                        return;
                    // Same detour rule as `openAlbum`: this is an album page too.
                    Backend.rememberAlbum(data.album.id);
                    nowPlayingExpanded = false;
                    var album = data.album;
                    var meta = [];
                    if (album.year)
                        meta.push(String(album.year));
                    if (album.tracks)
                        meta.push(album.tracks + " tracks");
                    contentStack.push(albumEntryComponent, {
                        albumId: album.id,
                        albumTitle: album.title,
                        albumArtist: album.artist ? album.artist : "",
                        albumCover: album.cover ? album.cover : "",
                        albumMeta: meta.join("   ·   ")
                    });
                });
                return;
            }
            // ARC-43: Go to Album / Go to Artist from the quick-action
            // overlay — navigation only, no queue change. ARC-I: a jump from a
            // search hit closes the search so the page is actually shown.
            if (channel === "open_album") {
                var page = JSON.parse(payload);
                Qt.callLater(function() {
                    if (searchOverlay.opened)
                        searchOverlay.close();
                    root.openAlbum(page.id, page.title, page.artist ? page.artist : "",
                                   page.year ? String(page.year) : "",
                                   String(page.tracks), page.cover ? page.cover : "",
                                   page.starred === true);
                });
                return;
            }
            if (channel === "open_artist") {
                var artist = JSON.parse(payload);
                Qt.callLater(function() {
                    if (searchOverlay.opened)
                        searchOverlay.close();
                    // The artist page is a shell surface: leaving the
                    // fullscreen is part of arriving (the queue's Alt "Go
                    // to Artist" must actually show the page).
                    root.nowPlayingExpanded = false;
                    contentStack.push(artistEntryComponent, {
                        artistId: artist.id,
                        artistName: artist.name,
                        artistCover: artist.cover ? artist.cover : ""
                    });
                });
            }
        }
    }

    // Region borders = the vi cursor. 1 = main, 2 = status (top); the
    // player bar has no region: it is decoration driven by global binds.
    Repeater {
        model: 2

        delegate: Rectangle {
            radius: Theme.radius
            id: regionBox
            required property int index

            readonly property int region: index + 1
            readonly property bool active: region === root.activeRegion
            readonly property real unit: Math.max(1, root.width * 0.0015)

            x: 0
            y: region === 2 ? 0 : root.barH
            width: root.width
            height: region === 2 ? root.barH : root.bodyH
            z: Theme.zRegion
            color: "transparent"
            // Only the active (cursor) region is outlined. Fullscreen is an
            // overlay, so no region highlight may show above it; the modal
            // search owns the focus border while it is up.
            visible: !root.nowPlayingExpanded && !searchOverlay.opened && active
            border.width: 2 * unit
            border.color: Palette.primary

            // Width is instant (R5); only the colour eases.
            Behavior on border.color {
                ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            }
        }
    }

    // Prime the status bar with the current counts once loaded.
    Timer {
        interval: 0
        running: true
        repeat: false
        onTriggered: Backend.refreshLibraryStatus()
    }

    // ARC-45: the travel log starts on the first place.
    Component.onCompleted: root.recordPlace()
}
