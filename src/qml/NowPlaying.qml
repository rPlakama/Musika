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
import QtQuick.Effects
import musika
import "ScrollMemory.js" as Memory
import "Theme.js" as Theme

// Fullscreen player ("mainscreen"), a 50/50 split:
//   left   — cover, identity, seek and transport (decoration: the transport
//            owns no keyboard pane, Space/n/p/,/. are global);
//   right  — Lyrics (top half) over Queue (bottom half), both always visible;
//   bottom — context-aware keybar for the focused pane (Tab cycles
//            lyrics/queue).
// Esc (or q) leaves fullscreen; Space stays the global play/pause.
Rectangle {
    radius: Theme.radius
    id: root

    color: Palette.background

    /// Click/tap on the exit button (the header chip); Home leaves fullscreen.
    signal exitRequested
    /// ARC-UI-1: Alt-hold quick actions on the queue row under the cursor;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    // UI-57: the track title ramps with the window width like the list
    // headers do (ScrollPanel), toward the headline floor — never below it.
    readonly property real widthScale: Theme.widthScale(root.width)
    /// Star hitbox side under the playing cover (five slots total).
    readonly property real starUnit: Math.max(14, Math.min(30, root.height * 0.045))
    readonly property real trackTitlePixelSize: {
        var full = Theme.fontSize("headline", root.height);
        return Theme.clamp(full * root.widthScale, 18, full);
    }

    // Keyboard pane: 1 = lyrics, 2 = queue, 3 = album cover actions.
    // Transport is never a pane.
    property int pane: 1
    property bool albumStarred: false
    // UI-60: the playing album's user rating (0..=5, 0 = unrated).
    property int albumRating: 0
    property bool albumCacheOn: false
    property int queueCursor: 0
    property int queueIndex: -1
    // One queue payload after a reorder keeps the moved cursor instead of
    // snapping back to the playing track (Q-9).
    property bool _reordering: false
    // The rows the queue model currently holds: a payload with the same
    // backend row hash is a skip/advance, not a queue change — no rebuild.
    property string _queueHash: ""
    // UI-54: first queue payload after entering restores the saved highlight.
    property bool _recallQueue: false

    // The active lyric glides to the pane centre through the shared scroll
    // modal (LYR-8): a soothing fixed-duration follow (`followMs`) inside the
    // centred band, never a teleport/flick. A manual scroll owns the pane
    // only until `lyricResume` fires: 5 s idle while lyrics is the focused
    // pane, 2 s when focus is elsewhere, so a stale line is never left
    // un-centred for long.
    property bool _lyricsHeld: false

    // LYR-8: resume the centre-follow after the manual-scroll idle window.
    Timer {
        id: lyricResume
        repeat: false
        onTriggered: {
            root._lyricsHeld = false;
            root.followLyric();
        }
    }

    // The seek row's timecode metric: the widest timecode the labels can
    // render ("00:00:00"), measured in the exact font the labels use. The
    // boxes below reserve that room, so digits ticking and tracks changing
    // never resize them (and never move the seek bar).
    FontMetrics {
        id: timeMetrics
        font.family: Theme.fonts.body
        font.pixelSize: Theme.fontSize("mono", root.height)
    }

    // Lyrics (override → cache → server; see src/lyrics.rs).
    property bool lyricsSynced: false
    property bool lyricsAvailable: false
    property bool lyricsLoading: false
    property string lyricsSource: ""
    property string lyricsLang: ""
    property int lyricsOffset: 0
    // Keep-alive: the song whose lyrics the model currently holds. Leaving
    // fullscreen keeps the lines, cursor and follow state; re-entry only
    // re-requests when the track changed while the player was closed (no
    // reload, no "Loading lyrics…" flash, no re-centring flick).
    property string _lyricsSongId: ""
    property var _lyricTimestamps: []

    // Lyrics type is sized from the pane itself (parent rectangle), never the
    // window, so lines can't outgrow the pane's walls; the stage caps the
    // active emphasis to the padded room, so the grown line never reaches the
    // margin (see the delegate's `emphasis`).
    readonly property real lyricSize: Theme.clamp(lyricsPane.height * 0.055, 10, 18)
    readonly property real lyricPad: Math.max(6, lyricsPane.height * 0.04)

    // Active synced line, matched against the offset-corrected position.
    // Zero-allocation: reads timestamps from flat JS array instead of querying
    // lyricModel.get(i) on every sub-second position update.
    readonly property int activeLyric: {
        var times = root._lyricTimestamps;
        var count = times.length;
        if (!lyricsSynced || count === 0)
            return -1;
        var position = Math.round(Backend.position * 1000) - lyricsOffset;
        // PATH-11: timestamps are ordered; binary search instead of walking
        // every line on each position tick.
        var low = 0;
        var high = count - 1;
        var active = -1;
        while (low <= high) {
            var mid = (low + high) >> 1;
            var start = times[mid];
            if (start <= position) {
                if (start >= 0)
                    active = mid;
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
        return active;
    }

    function keyHints() {
        if (pane === 1)
            return "j/k scroll   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab queue   ·   Esc exit";
        if (pane === 3)
            return "Enter cover actions   ·   Tab lyrics   ·   Esc exit";
        return "j/k select   ·   Shift-J/K move   ·   Enter jump   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab lyrics   ·   Esc exit";
    }

    /// The keybar's album-scope state (favourite + container download) for
    /// the playing track's album.
    function requestAlbumActions() {
        if (Backend.currentAlbumId.length === 0)
            return;
        Backend.requestAlbumCache(Backend.currentAlbumId);
        Backend.requestAlbumState(Backend.currentAlbumId);
    }

    function albumItem() {
        return {
            albumId: Backend.currentAlbumId,
            starred: root.albumStarred,
            rating: root.albumRating
        };
    }

    /// The big cover owns the album verbs (favourite + count + download).
    function coverActions() {
        return verbs.album(root.albumItem(), root.albumCacheOn, function(level) {
            root.albumRating = level;
        });
    }

    /// The state marks render only inside the queue cursor's ±15-row
    /// window — long queues keep the bindings cheap.
    function markVisible(index) {
        return Math.abs(index - Math.max(0, root.queueCursor)) <= 15;
    }

    /// Ask the backend for the download state of that window only.
    function requestMarkWindow(force) {
        var ids = [];
        var at = Math.max(0, root.queueCursor);
        var lo = Math.max(0, at - 15);
        var hi = Math.min(queueItems.count - 1, at + 15);
        for (var i = lo; i <= hi; ++i) {
            var row = queueItems.get(i);
            if (row && row.songId)
                ids.push(row.songId);
        }
        if (force)
            Backend.refreshDownloads(JSON.stringify(ids), "[]");
        else
            Backend.requestDownloads(JSON.stringify(ids), "[]");
    }

    onQueueCursorChanged: root.requestMarkWindow()

    /// The playing track's row in the queue model (song id + favourite flag
    /// for the overlay's track-scoped options).
    function currentTrack() {
        return root.queueIndex >= 0 && root.queueIndex < queueItems.count
            ? queueItems.get(root.queueIndex)
            : null;
    }

    // ── Row download marks: the global ledger property --------------------
    // `Backend.downloadedIds` (download.db's finished songs) is the single
    // truth every row binds to by song id — the download twin of the hearts'
    // library read. Refreshed on entry and after container changes.

    /// The favourite heart, patched in place on the acted row (the playing
    /// track the keybar verbs target). QR-8: no whole-model scan — duplicate
    /// rows reconcile on the next queue payload.
    function markRowStarred(songId, on) {
        var i = root.queueIndex;
        if (i >= 0 && i < queueItems.count && queueItems.get(i).songId === songId)
            queueItems.setProperty(i, "starred", on === true);
    }

    function scrollLyrics(delta) {
        root.holdLyrics();
        var view = lyricsView.view;
        // Origin-aware range: the unlimited-scrolloff spacers sit above and
        // below the lines, so the manual scroll reaches them too (originY is
        // negative with the top spacer).
        var lo = Math.min(0, view.originY);
        var limit = Math.max(lo, lo + view.contentHeight - view.height);
        view.contentY = Math.max(lo, Math.min(limit, view.contentY + delta));
    }

    function holdLyrics() {
        root._lyricsHeld = true;
        lyricsView.stopGlide();
        lyricResume.interval = root.pane === 1 ? Theme.lyricsResume : Theme.lyricsResumeUnfocused;
        lyricResume.restart();
    }

    /// The shared body owns the motion: pushing the active line makes the
    /// view glide it to the pane centre over `followMs`.
    function followLyric() {
        if (root._lyricsHeld || !lyricsSynced || activeLyric < 0 || lyricModel.count === 0)
            return;
        lyricsView.currentIndex = root.activeLyric;
    }

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }

    function moveQueueCursor(delta) {
        if (queueItems.count === 0)
            return;
        queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor + delta));
        // The view follows cursor changes itself (SCROLLCONTENTSTUDY.md).
        queueContent.currentIndex = queueCursor;
    }

    // VI-4: visual selection over the fullscreen queue. The state lives in
    // `queueContent` (the shared scroll model); Home routes `v`/`V`/Esc here
    // while the player is up.
    readonly property bool hasSelection: queueContent.hasSelection
    readonly property int selectionCount: queueContent.selectionCount

    function toggleSelection() {
        queueContent.toggleSelection();
    }

    function selectAll() {
        if (queueItems.count === 0)
            return;
        queueCursor = queueItems.count - 1;
        queueContent.currentIndex = queueCursor;
        queueContent.selectAllRows();
        queueContent.goto(queueCursor);
    }

    function clearSelection() {
        queueContent.clearSelection();
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`) — the
    /// fullscreen surface is its queue pane, so a jump focuses it and the
    /// context-Enter that follows lands on the matched row.
    property alias searchQuery: queueContent.searchQuery
    property alias searchCurrentRow: queueContent.searchCurrentRow

    function searchRows() {
        return queueContent.searchRows();
    }

    function searchJump(row) {
        pane = 2;
        queueContent.searchJump(row);
        queueCursor = queueContent.currentIndex;
    }

    function applyQueue(payload) {
        var data = JSON.parse(payload);

        // A skip or an auto-advance re-delivers the exact same rows with a
        // new playing index. Rebuilding the model for that flicked the pane
        // (clear + fade) and, when the follow raced the refill, corrupted
        // the view's layout (realised rows stopped at the old window and
        // later jumps inflated the item offsets — SC-5). So: rebuild only
        // when the rows really changed; a pure move just follows the index.
        if (data.hash !== root._queueHash) {
            root._queueHash = data.hash;
            // ARC-memory: a different row set is a new queue — drop the old
            // queue's remembered scroll/prefetch target before it can be
            // restored.
            Memory.clearScroll("fullscreen");
            root._recallQueue = false;
            // UI-41: payload swaps fade in quickly; a reorder keeps the list live.
            if (!root._reordering)
                queueContent.opacity = 0;
            queueItems.clear();
            for (var i = 0; i < data.items.length; i++) {
                var track = data.items[i];
                queueItems.append({
                    title: track.title,
                    duration: Theme.formatTime(track.duration),
                    songId: track.id,
                    albumId: track.album_id ? track.album_id : "",
                    starred: track.starred === true
                });
            }
            Qt.callLater(function () {
                queueContent.opacity = 1;
            });
        }

        queueIndex = data.index;
        if (root._reordering) {
            root._reordering = false;
            root._recallQueue = false;
            queueCursor = Math.max(0, Math.min(queueItems.count - 1, queueCursor));
            queueContent.currentIndex = queueCursor;
            queueContent.goto(queueCursor);
        } else {
            root.recallQueueCursor();
        }
        root.requestMarkWindow();
    }

    /// The queue cursor for a fresh payload (UI-54 re-entry remembers; a
    /// live payload follows the playing track). Shared by `applyQueue` and
    /// the keep-alive re-entry in `onVisibleChanged`.
    function recallQueueCursor() {
        if (root._recallQueue && queueContent.restore()) {
            root._recallQueue = false;
            queueCursor = queueContent.currentIndex;
            return;
        }
        root._recallQueue = false;
        queueCursor = Math.max(0, Math.min(queueIndex, Math.max(0, queueItems.count - 1)));
        queueContent.currentIndex = queueCursor;
        queueContent.goto(queueCursor);
    }

    function applyLyrics(payload) {
        var data = JSON.parse(payload);
        if (data.song_id !== Backend.currentSongId)
            return; // stale response for a track we already left

        root._lyricsSongId = data.song_id;
        lyricsSource = data.source;
        lyricsSynced = data.synced;
        lyricsLang = data.lang ? data.lang : "";
        lyricsOffset = data.offset_ms ? data.offset_ms : 0;
        lyricsLoading = false;

        // Quick fade so a track change does not snap the lines (LYR-9).
        lyricsView.opacity = 0;
        lyricModel.clear();
        var times = [];
        for (var i = 0; i < data.lines.length; i++) {
            var tVal = data.lines[i].t === null || data.lines[i].t === undefined ? -1 : data.lines[i].t;
            times.push(tVal);
            lyricModel.append({
                t: tVal,
                line: data.lines[i].text
            });
        }
        root._lyricTimestamps = times;
        lyricsAvailable = lyricModel.count > 0 && data.source !== "none";

        lyricResume.stop();
        // A new song carries no cursor: clear it, then centre the opening
        // line right away — with unlimited scrolloff even row 0 sits at the
        // pane centre (empty space above it) — and let the first
        // `followLyric` glide take over once the active phrase moves.
        lyricsView.stopGlide();
        lyricsView.currentIndex = -1;
        lyricsView.goto(0);
        root._lyricsHeld = false;
        Qt.callLater(function () {
            lyricsView.opacity = 1;
            root.followLyric();
        });
    }

    function lyricsTag() {
        if (!lyricsAvailable)
            return "";
        var tag = lyricsSource.length > 0 ? lyricsSource : "";
        // `xxx`/`und` mean "unspecified" in OpenSubsonic; never show them.
        if (lyricsLang.length > 0 && lyricsLang !== "xxx" && lyricsLang !== "und")
            tag += " · " + lyricsLang;
        return tag;
    }

    onActiveLyricChanged: root.followLyric()

    // LYR-8: re-arm the idle window when pane focus moves (5 s vs 2 s).
    onPaneChanged: if (root._lyricsHeld) root.holdLyrics()

    /// ARC-UI-1: Alt armed the gesture — anchor on the queue row under the
    /// cursor; the fullscreen queue reuses the shared track verb set.
    function armQuickActions() {
        // UI-60: with the cover pane focused, Alt anchors on the big album
        // cover (the keybar chips are gone); its album verbs carry the
        // favourite + count + download.
        if (root.pane === 3) {
            if (Backend.currentAlbumId.length > 0)
                root.quickAction(coverBox, root.coverActions());
            return;
        }
        if (queueItems.count === 0 || queueContent.currentIndex < 0)
            return;
        var item = queueItems.get(queueContent.currentIndex);
        var row = queueContent.view.itemAtIndex(queueContent.currentIndex);
        var actions = root.hasSelection
            ? verbs.forSelection("song", queueContent.selectionIds("songId"),
                queueContent.selectionStarred("songId"))
            : (item ? verbs.song(item) : null);
        if (!actions)
            return;
        // ARC-I-4: the heart tracks the verb immediately (see `MediaList`).
        if (actions.south && actions.south.invoke) {
            var want = actions.south.favorite === true;
            var star = actions.south.invoke;
            actions.south.invoke = function() {
                star();
                queueContent.markStarred(want);
            };
        }
        if (row)
            quickAction(row, actions);
    }

    // Fullscreen keys, scoped to the focused pane. Esc/back stays with Home.
    function handleAction(action) {
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            // Tab cycles lyrics → queue → actions; Shift+Tab reverses.
            pane = action === "pane_cycle"
                ? (pane === 1 ? 2 : (pane === 2 ? 3 : 1))
                : (pane === 3 ? 2 : (pane === 2 ? 1 : 3));
            return;
        }
        if (action === "next_track") {
            Backend.playerNext();
            return;
        }
        if (action === "prev_track") {
            Backend.playerPrevious();
            return;
        }

        if (pane === 1) {
            // VI-7: the count scales the lyric scroll.
            var lyricSteps = Math.max(1, Backend.keyCount);
            if (action === "move_up")
                scrollLyrics(-lyricsView.height * 0.3 * lyricSteps);
            else if (action === "move_down")
                scrollLyrics(lyricsView.height * 0.3 * lyricSteps);
            else if (action === "move_left" || action === "move_right")
                pane = 2;
            return;
        }

        if (pane === 3) {
            // The big cover is the action surface: Enter arms its album
            // verbs (favourite, count, download) as a sticky menu.
            if (action === "enter" && Backend.currentAlbumId.length > 0) {
                var coverMenu = root.coverActions();
                coverMenu.sticky = true;
                root.quickAction(coverBox, coverMenu);
            }
            return;
        }

        // VI-7: the count walks the queue cursor by that many rows.
        var queueSteps = Math.max(1, Backend.keyCount);
        if (action === "top" || action === "bottom") {
            queueContent.edgeJump(action === "bottom", queueSteps);
            queueCursor = queueContent.currentIndex;
            return;
        }
        if (action === "move_up")
            moveQueueCursor(-queueSteps);
        else if (action === "move_down")
            moveQueueCursor(queueSteps);
        else if (action === "queue_up" && queueCursor > 0) {
            root._reordering = true;
            // VI-4: a reorder shifts every anchored row; the range dies.
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor - 1);
            queueCursor -= 1;
            queueContent.currentIndex = queueCursor;
            return;
        } else if (action === "queue_down" && queueCursor < queueItems.count - 1) {
            root._reordering = true;
            queueContent.clearSelection();
            Backend.queueMove(queueCursor, queueCursor + 1);
            queueCursor += 1;
            queueContent.currentIndex = queueCursor;
            return;
        } else if (action === "enter") {
            // VI-4: jumping to a track ends the visual selection.
            queueContent.clearSelection();
            if (queueCursor >= 0)
                Backend.queueJump(queueCursor);
        } else if (action === "move_left" || action === "move_right")
            pane = 1;
    }

    onVisibleChanged: {
        if (visible) {
            // UI-54: re-entering restores the last fullscreen scope (pane) and
            // queue highlight; the first queue payload lands them.
            var saved = Memory.loadScroll("fullscreen");
            pane = saved && saved.scope >= 1 ? saved.scope : 1;
            root._recallQueue = true;
            // Queue and lyrics stay alive while the player is closed: the
            // queue model (fed by the hidden payloads) only needs the first
            // ever fetch, and the highlight is recalled from the live model.
            // No re-request, no rebuild, no fade on every enter.
            if (queueItems.count === 0)
                Backend.requestQueue();
            else
                root.recallQueueCursor();
            // Lyrics are the same: fetch only when the track changed behind
            // the curtain, otherwise the lines stay exactly where they were
            // (no reload, no flick).
            if (root._lyricsSongId !== Backend.currentSongId) {
                lyricsLoading = true;
                Backend.requestLyrics();
            }
            // The keybar's album actions reconcile on entry too.
            root.requestAlbumActions();
            // The row download marks read only the visible window.
            root.requestMarkWindow();
        } else if (queueItems.count > 0) {
            queueContent.remember(pane);
            // VI-4: leaving fullscreen ends the queue selection.
            queueContent.clearSelection();
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            // The queue applies even while hidden: the backend emits on every
            // mutation and track advance, so the model is already current
            // when the player reopens (no reload on entry).
            if (channel === "queue")
                root.applyQueue(payload);
            else if (channel === "lyrics" && root.visible)
                root.applyLyrics(payload);
            else if (channel === "album_cache") {
                var cache = JSON.parse(payload);
                if (cache.id === Backend.currentAlbumId) {
                    root.albumCacheOn = cache.on === true;
                }
                // The pass' row updates travel per-id (the batch kick plus
                // each song's completion); only an unselect must hard-reset
                // the accumulated windows and re-request.
                if (!cache.active && cache.on !== true)
                    Backend.ResetRowMarks();
            } else if (channel === "artist_cache") {
                // Artist passes settle outside the keybar; unselect resets.
                var artistCache = JSON.parse(payload);
                if (!artistCache.active && artistCache.on !== true)
                    Backend.ResetRowMarks();
            } else if (channel === "album_state") {
                var state = JSON.parse(payload);
                if (state.id === Backend.currentAlbumId) {
                    root.albumStarred = state.starred === true;
                    root.albumRating = Number(state.rating) || 0;
                }
            }
        }
        function onCurrent_album_id_changed() {
            if (root.visible)
                root.requestAlbumActions();
        }
        // A fetch finished: re-read the queue's own window (forced).
        function onRowReload() {
            root.requestMarkWindow(true);
        }
        // A favourite landed (per row, no payload): patch every matching
        // queue row in place. The cursor stays where the user left it — a
        // heart write must never re-follow the playing track.
        function onFavoriteChanged(kind, id, on) {
            if (kind !== "song")
                return;
            for (var i = 0; i < queueItems.count; ++i) {
                if (queueItems.get(i).songId === id)
                    queueItems.setProperty(i, "starred", on === true);
            }
        }
        // A skip/advance is deduped backend-side (the rows are already
        // published): the index property carries the move — adopt it and
        // follow exactly like a fresh payload would.
        function onQueue_index_changed() {
            if (root.queueIndex === Backend.queueIndex)
                return;
            root.queueIndex = Backend.queueIndex;
            root.recallQueueCursor();
        }
        function onCurrent_song_id_changed() {
            if (root.visible) {
                root.lyricsLoading = true;
                Backend.requestLyrics();
            }
        }
        // LYR-8: a seek (keys, seek bar, MPRIS) must not wait out the manual-
        // scroll resume window — drop the hold and re-centre at once, so the
        // lyrics land with the progress jump instead of 3/5 s later.
        function onSeeked() {
            root._lyricsHeld = false;
            lyricResume.stop();
            root.followLyric();
        }
    }

    ListModel {
        id: queueItems
    }

    // ARC-UI-1: the recycled Alt verb set; fullscreen queue rows are tracks.
    QuickActions {
        id: verbs
    }

    ListModel {
        id: lyricModel
    }

    // D-5 ambience: the playing cover through the shared `CoverImage.ambient`
    // mode (blurred and washed into the theme background, same rule as
    // AlbumEntry), so the square floats on its own colours without competing
    // with the lyrics/queue panes. Hidden without art; the panes stay opaque.
    // UI-55: one item for cover + wash, so the panes' acrylic samples exactly
    // what is painted behind them.
    Item {
        id: ambience
        anchors.fill: parent

        CoverImage {
            id: coverBackdrop
            anchors.fill: parent
            source: Backend.currentCoverUrl
            ambient: true
        }

        Rectangle {
            radius: Theme.radius
            anchors.fill: parent
            visible: coverBackdrop.visible
            gradient: Gradient {
                GradientStop {
                    position: 0.0
                    color: root.withAlpha(Palette.background, 0.84)
                }
                GradientStop {
                    position: 0.38
                    color: root.withAlpha(Palette.background, 0.38)
                }
                GradientStop {
                    position: 1.0
                    color: root.withAlpha(Palette.background, 0.96)
                }
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.025
        spacing: root.height * 0.018

        // Header: track title + exit hint.
        RowLayout {
            Layout.fillWidth: true

            FadeText {
                Layout.fillWidth: true
                text: "MUSIKA"
                color: Palette.onSurface
                font.family: Theme.fonts.wordmark
                font.pixelSize: Theme.fontSize("title", root.height)
                font.bold: true
                elide: Text.ElideRight
            }

            // Exit chip: same shared control as the album/artist headers.
            ExitChip {
                host: root
                label: "ESC/Q"
                padRatio: 0.02
                onClicked: root.exitRequested()
            }
        }

        // Body: left (cover + transport) and right (lyrics over queue), 50/50.
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: root.width * 0.02

            // ---- Left column: cover, identity, seek, transport ----
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: root.width * 0.48
                spacing: root.height * 0.012

                // D-3 cover rule, shared with AlbumEntry: a square sized by
                // the available box and the source ceiling (512), centered in
                // the flexible area. Width never stretches the container, so
                // no window shape crops the art.
                Item {
                    id: coverArea
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    Column {
                        anchors.centerIn: parent
                        spacing: root.height * 0.008

                        Rectangle {
                            id: coverBox
                            readonly property real side: Math.max(96, Math.min(root.height * 0.42, 512, coverArea.width, coverArea.height - root.starUnit - root.height * 0.008))
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: side
                            height: side
                            radius: Theme.radius
                            color: Palette.primaryContainer
                            clip: true

                            CoverImage {
                                anchors.fill: parent
                                source: Backend.currentCoverUrl
                            }

                            FadeText {
                                anchors.centerIn: parent
                                text: "MUSIKA"
                                color: Palette.onPrimaryContainer
                                font.family: Theme.fonts.wordmark
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.letterSpacing: root.height * 0.006
                                visible: Backend.currentCoverUrl.length === 0
                            }

                            // UI-60: the cover is the album action surface.
                            ScopeRing {
                                shown: root.pane === 3
                            }

                            HoverHandler {
                                cursorShape: Qt.PointingHandCursor
                            }

                            TapHandler {
                                onTapped: {
                                    root.pane = 3;
                                    if (Backend.currentAlbumId.length === 0)
                                        return;
                                    var menu = root.coverActions();
                                    menu.sticky = true;
                                    root.quickAction(coverBox, menu);
                                }
                            }
                        }

                        StarRating {
                            anchors.horizontalCenter: parent.horizontalCenter
                            unit: root.starUnit
                            rating: root.albumRating
                            onPicked: (level) => {
                                root.albumRating = level;
                                if (Backend.currentAlbumId.length > 0)
                                    Backend.rateAlbum(Backend.currentAlbumId, level);
                            }
                        }
                    }
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentArtist
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "No track playing"
                    color: Palette.onSurface
                    font.pixelSize: root.trackTitlePixelSize
                    font.bold: true
                    elide: Text.ElideRight
                }

                FadeText {
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0
                }

                // Seek row: elapsed, bar, duration. The row fills whatever
                // the column offers — the bar freely takes every pixel the
                // numbers leave (the fixed boxes only hold the shake still,
                // they never shrink the bar's room).
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.width * 0.004

                    // The SuperiorBar trick: the timecode sits in its own
                    // fixed-width rectangle (reserved for the widest string),
                    // so the ticking position repaints inside it and the bar
                    // next to it keeps a steady length. The digits hug the
                    // box's left edge — the seek row's first mark lines up
                    // with the title and the artist above it.
                    Rectangle {
                        radius: Theme.radius
                        Layout.preferredWidth: timeMetrics.advanceWidth("00:00:00")
                        Layout.preferredHeight: elapsedText.implicitHeight
                        Layout.alignment: Qt.AlignVCenter
                        color: "transparent"

                        FadeText {
                            id: elapsedText
                            anchors.fill: parent
                            text: Theme.formatTime(Backend.position)
                            color: Palette.onSurfaceVariant
                            font.pixelSize: Theme.fontSize("mono", root.height)
                            horizontalAlignment: Text.AlignLeft
                            animated: false
                        }
                    }

                    Rectangle {
                        id: seekTrack
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(2, root.height * 0.006)
                        color: Palette.outlineVariant
                        radius: Theme.radius

                        Rectangle {
                            width: seekTrack.width * (Backend.duration > 0 ? Math.min(1, Backend.position / Backend.duration) : 0)
                            height: parent.height
                            color: Palette.primary
                            radius: Theme.radius

                            Behavior on width {
                                NumberAnimation {
                                    duration: Theme.motion.quick
                                    easing.type: Easing.Linear
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onPressed: mouse => {
                                if (Backend.duration > 0)
                                    Backend.playerSeek(Math.max(0, Math.min(1, mouse.x / width)) * Backend.duration);
                            }
                        }
                    }

                    Rectangle {
                        radius: Theme.radius
                        Layout.preferredWidth: timeMetrics.advanceWidth("00:00:00")
                        Layout.preferredHeight: durationText.implicitHeight
                        Layout.alignment: Qt.AlignVCenter
                        color: "transparent"

                        FadeText {
                            id: durationText
                            anchors.fill: parent
                            text: Theme.formatTime(Backend.duration)
                            color: Palette.onSurfaceVariant
                            font.pixelSize: Theme.fontSize("mono", root.height)
                            horizontalAlignment: Text.AlignLeft
                            animated: false
                        }
                    }
                }

                // Transport bar: mouse-only decoration (UI-51; no focus
                // hairline, Space/n/p/,/. are global). One major rectangle
                // owns both clusters as their own rectangles: the volume box
                // (right) yields its bar width first, and the transport box
                // centres in the whole bar while there is room, sliding left
                // only as far as needed once the volume box would touch it —
                // the transport yields, the volume stays put.
                Rectangle {
                    id: transportBar
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.height * 0.07
                    color: "transparent"
                    radius: Theme.radius

                    // Volume cluster (P-7): tiered icon (click = mute) + bar.
                    // The bar is the elastic part; the box clamps to whatever
                    // the transport is not using.
                    Rectangle {
                        radius: Theme.radius
                        id: volumeBox
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.max(root.height * 0.04,
                            Math.min(volumeRow.implicitWidth,
                                transportBar.width - transportBox.width - root.width * 0.012))
                        height: parent.height
                        color: "transparent"

                        RowLayout {
                            id: volumeRow
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width
                            spacing: root.width * 0.008

                            IconButton {
                                Layout.preferredWidth: root.height * 0.04
                                Layout.preferredHeight: root.height * 0.04
                                iconSource: Theme.volumeIcon(Backend.volume, Backend.muted)
                                symbolRatio: 0.95
                                onClicked: Backend.playerToggleMute()
                            }

                            Rectangle {
                                id: volumeTrack
                                Layout.fillWidth: true
                                Layout.minimumWidth: root.height * 0.02
                                Layout.maximumWidth: root.width * 0.06
                                Layout.preferredWidth: root.width * 0.06
                                Layout.preferredHeight: Math.max(2, root.height * 0.006)
                                color: Palette.outlineVariant
                                radius: Theme.radius

                                Rectangle {
                                    width: parent.width * Math.max(0, Math.min(1, Backend.volume / 100))
                                    height: parent.height
                                    radius: Theme.radius
                                    color: Backend.muted ? Palette.outline : Palette.primary

                                    Behavior on width {
                                        NumberAnimation {
                                            duration: Theme.motion.quick
                                            easing.type: Easing.Linear
                                        }
                                    }
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onPressed: mouse => Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100)
                                    onPositionChanged: mouse => {
                                        if (pressed)
                                            Backend.playerSetVolume(Math.max(0, Math.min(1, mouse.x / width)) * 100);
                                    }
                                }
                            }
                        }
                    }

                    // Transport cluster (skip / pause / next) in its own
                    // rectangle: centred in the whole bar while there is room,
                    // and pushed left only as far as needed once the volume
                    // box would touch it — the transport yields, the volume
                    // stays put.
                    Rectangle {
                        radius: Theme.radius
                        id: transportBox
                        anchors.verticalCenter: parent.verticalCenter
                        width: transportRow.implicitWidth
                        height: parent.height
                        color: "transparent"
                        x: Math.max(0, Math.min(
                            (transportBar.width - width) / 2,
                            transportBar.width - volumeBox.width
                                - root.width * 0.012 - width))

                        // UI-59: the shared transport cluster (same one as
                        // the player bar).
                        TransportControls {
                            id: transportRow
                            anchors.centerIn: parent
                            unit: root.height * 0.05
                        }
                    }
                }
            }

            // ---- Right column: Lyrics (top) over Queue (bottom) ----
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.preferredWidth: root.width * 0.48
                spacing: root.height * 0.012

                // Lyrics, top half. UI-55: the solid pane over the ambient
                // blur becomes acrylic (blurred backdrop + tint + grain).
                AcrylicPanel {
                    id: lyricsPane
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    tint: Palette.surfaceContainerLow
                    backdrop: coverBackdrop.visible ? ambience : null
                    radius: Theme.radius
                    border.width: root.pane === 1 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Palette.primary

                    TapHandler {
                        onTapped: root.pane = 1
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: root.width * 0.012
                        spacing: root.height * 0.006

                        RowLayout {
                            Layout.fillWidth: true

                            FadeText {
                                text: "Lyrics"
                                color: root.pane === 1 ? Palette.primary : Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            FadeText {
                                text: root.lyricsTag()
                                color: Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }

                            // Synced as a small MY3 chip in `label` type.
                            Rectangle {
                                visible: root.lyricsAvailable && root.lyricsSynced
                                implicitWidth: syncedLabel.implicitWidth + root.height * 0.02
                                implicitHeight: root.height * 0.026
                                radius: Theme.radius
                                color: Palette.secondaryContainer

                                FadeText {
                                    id: syncedLabel
                                    anchors.centerIn: parent
                                    text: "SYNCED"
                                    color: Palette.onSecondaryContainer
                                    font.pixelSize: Theme.fontSize("label", root.height)
                                    font.letterSpacing: root.height * 0.001
                                    animated: false
                                }
                            }
                        }

                        Rectangle {
                            radius: Theme.radius
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 1 ? Palette.primary : Palette.outlineVariant
                        }

                        // The stage: an inner rectangle inside the pane that
                        // contains every lyric line, the scaled active one
                        // included. Delegates pre-calculate their emphasis so
                        // the grown line never reaches these walls; `clip` is
                        // the last-resort containment.
                        Rectangle {
                            radius: Theme.radius
                            id: lyricsStage
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            color: "transparent"
                            clip: true

                            ScrollContent {
                                id: lyricsView
                                anchors.fill: parent
                                visible: root.lyricsAvailable
                                model: lyricModel
                                rowSpacing: root.lyricSize * 0.9
                                // Unlimited scrolloff: the active phrase is
                                // always centred, the opening and closing
                                // lines included — half a pane of empty
                                // space sits above row 0 and below the last.
                                unlimitedScrolloff: true
                                // LYR-8: the soothing lyrics follow — the
                                // shared glide at 300 ms that centres the
                                // phrase with the OutBack spring. The pane's
                                // fade rides the same value.
                                followMs: 300

                                // A hand on the pane owns it until the next
                                // track.
                                Connections {
                                    target: lyricsView.view
                                    function onMovementStarted() { root.holdLyrics(); }
                                }

                                // LYR-8: the fade in/out shares the follow's
                                // transition value, so a new song arrives and
                                // settles as one motion.
                                Behavior on opacity {
                                    NumberAnimation {
                                        duration: lyricsView.followMs
                                        easing.type: Easing.Bezier
                                        easing.bezierCurve: Theme.ease.decelerate
                                    }
                                }

                                rowDelegate: Component {
                                    FadeText {
                                        id: lyricLine
                                        required property int index
                                        required property real t
                                        required property string line

                                        // Lyrics stage (roadmap "Lyrics presentation"):
                                        // active = scale + full contrast, near = dim,
                                        // far = dimmer; unsynced is one uniform column.
                                        readonly property bool active: index === root.activeLyric
                                        readonly property int distance: root.activeLyric >= 0 ? Math.abs(index - root.activeLyric) : -1

                                        // Pre-calculated emphasis: the full 1.12
                                        // while the wrapped line stays clear of the
                                        // margin, otherwise exactly the scale that
                                        // fits between the paddings (never touching
                                        // the stage walls). Long unbreakable words
                                        // can shrink below 1 instead of being cut.
                                        readonly property real emphasis: {
                                            if (!lyricLine.active)
                                                return 1.0;
                                            var room = lyricLine.width - lyricLine.leftPadding - lyricLine.rightPadding;
                                            if (room <= 0 || lyricLine.paintedWidth <= 0)
                                                return Theme.lyricsEmphasis;
                                            return Math.min(Theme.lyricsEmphasis, room / lyricLine.paintedWidth);
                                        }
                                        // Grow the slot symmetrically so the big
                                        // line stays centred and its neighbours
                                        // make room instead of getting overlapped.
                                        readonly property real emphasisPad: Math.max(0, lyricLine.paintedHeight * (lyricLine.emphasis - 1) / 2)

                                        width: lyricsView.width
                                        leftPadding: root.lyricPad
                                        rightPadding: root.lyricPad
                                        topPadding: lyricLine.emphasisPad
                                        bottomPadding: lyricLine.emphasisPad
                                        text: lyricLine.line
                                        horizontalAlignment: Text.AlignHCenter
                                        color: lyricLine.active ? Palette.onSurface : Palette.onSurfaceVariant
                                        font.family: Theme.fonts.lyrics
                                        font.pixelSize: root.lyricSize
                                        wrapMode: Text.WordWrap
                                        animated: false
                                        transformOrigin: Item.Center
                                        opacity: lyricLine.distance < 0 ? Theme.contentDim : (lyricLine.active ? 1.0 : (lyricLine.distance === 1 ? Theme.contentDim : Theme.contentDimFar))
                                        scale: lyricLine.emphasis

                                        Behavior on opacity {
                                            NumberAnimation {
                                                duration: Theme.motion.quick
                                                easing.type: Easing.Bezier
                                                easing.bezierCurve: Theme.ease.standard
                                            }
                                        }
                                        Behavior on scale {
                                            NumberAnimation {
                                                duration: Theme.motion.quick
                                                easing.type: Easing.Bezier
                                                easing.bezierCurve: Theme.ease.standard
                                            }
                                        }
                                        Behavior on color {
                                            ColorAnimation {
                                                duration: Theme.motion.micro
                                                easing.type: Easing.Bezier
                                                easing.bezierCurve: Theme.ease.standard
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: root.lyricsLoading
                            text: "Loading lyrics…"
                            color: Palette.onSurfaceVariant
                            font.family: Theme.fonts.lyrics
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: !root.lyricsAvailable && !root.lyricsLoading
                            text: "No lyrics"
                            color: Palette.onSurfaceVariant
                            font.family: Theme.fonts.lyrics
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }

                // Queue, bottom half. UI-55: acrylic like the lyrics pane.
                AcrylicPanel {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    tint: Palette.surfaceContainerLow
                    backdrop: coverBackdrop.visible ? ambience : null
                    radius: Theme.radius
                    border.width: root.pane === 2 ? Math.max(1, root.width * Theme.hairlineRatio) : 0
                    border.color: Palette.primary
                    clip: true

                    TapHandler {
                        onTapped: root.pane = 2
                    }

                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: root.width * 0.012
                        spacing: root.height * 0.006

                        RowLayout {
                            Layout.fillWidth: true

                            FadeText {
                                text: "Queue"
                                color: root.pane === 2 ? Palette.primary : Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("title", root.height)
                                font.bold: true
                            }

                            Item {
                                Layout.fillWidth: true
                            }

                            // VI-4: the selection count while a range is up.
                            SelectionBadge {
                                Layout.alignment: Qt.AlignVCenter
                                count: root.selectionCount
                                pixelSize: Theme.fontSize("label", root.height)
                            }

                            FadeText {
                                text: Backend.queueLength > 0 ? (Backend.queueIndex + 1) + " / " + Backend.queueLength : ""
                                color: Palette.onSurfaceVariant
                                font.pixelSize: Theme.fontSize("label", root.height)
                            }
                        }

                        Rectangle {
                            radius: Theme.radius
                            Layout.fillWidth: true
                            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                            color: root.pane === 2 ? Palette.primary : Palette.outlineVariant
                        }

                        Item {
                            Layout.fillWidth: true
                            Layout.fillHeight: true

                            // UI-48: the shared scroll body carries the
                            // overflow stage — the highlight bleeds into the
                            // gutters while rows still clip vertically at the
                            // list bounds.
                            ScrollContent {
                                id: queueContent
                                anchors.fill: parent
                                bleed: width * 0.012
                                model: queueItems
                                selectable: true
                                memoryKey: "fullscreen"
                                // ARC-UI-2 M1: the queue rows paint title
                                // and duration.
                                searchFields: ["title", "duration"]
                                // UI-58: adaptive rows for the queue pane.
                                rowRatio: 0.16
                                // ARC-IDK: constant-height queue rows can
                                // pre-calculate their scroll extent before
                                // Qt's lazy delegate estimate settles.
                                contentHeightHint: queueItems.count
                                    * queueContent.adaptiveRowHeight
                                // The fullscreen memory also stores the focused
                                // pane; without this the continuous remembers
                                // clobbered it to the default scope.
                                memoryScope: root.pane
                                visible: queueItems.count > 0

                                Behavior on opacity {
                                    NumberAnimation {
                                        duration: Theme.motion.quick
                                        easing.type: Easing.Bezier
                                        easing.bezierCurve: Theme.ease.standard
                                    }
                                }

                                rowDelegate: Component {
                                    Rectangle {
                                        radius: Theme.radius
                                        id: row
                                        required property int index
                                        required property string title
                                        required property string duration
                                        required property string songId
                                        required property bool starred

                                        readonly property bool emphasized: index === root.queueCursor || rowHover.hovered

                                        width: ListView.view.width
                                        // UI-58: adaptive — cut a row past the
                                        // 18% squeeze instead of shrinking.
                                        height: queueContent.adaptiveRowHeight > 0
                                            ? queueContent.adaptiveRowHeight
                                            : ListView.view.height * 0.16
                                        // Playing-track tint stays on the row; the
                                        // cursor/hover block is the shared UI-48 layer.
                                        color: index === root.queueIndex ? Palette.surfaceContainer : "transparent"

                                        RowHighlight {
                                            current: index === root.queueCursor
                                            hovered: rowHover.hovered
                                            selected: queueContent.selected(index)
                                            overflow: queueContent.bleed
                                        }

                                        RowBody {
                                            anchors.fill: parent
                                            viewWidth: row.width
                                            rowHeight: row.height
                                            emphasized: row.emphasized
                                            showCover: false
                                            showSubtitle: false
                                            title: row.title
                                            meta: row.duration
                                            titlePixelSize: Theme.fontSize("body", root.height)
                                            titleColor: index === root.queueCursor ? Palette.onSecondaryContainer : Palette.onSurface
                                            metaPixelSize: Theme.fontSize("mono", root.height)
                                            reserveMeta: true
                                            metaAlignRight: true
                                            leftMargin: row.width * 0.01
                                            textGap: row.width * 0.01
                                            rightMargin: 0
                                            marksMargin: row.width * 0.00935
                                            markCursor: root.queueCursor
                                            markIndex: row.index
                                            markSongId: row.songId
                                            markStarred: row.starred
                                            searchQuery: queueContent.searchQuery
                                            searchCurrent: index === queueContent.searchCurrentRow
                                            expected: row.index !== queueContent.currentIndex
                                                && queueContent.isExpected(row.y)
                                            immediate: row.index === queueContent.currentIndex
                                        }

                                        HoverHandler {
                                            id: rowHover
                                            cursorShape: Qt.PointingHandCursor
                                        }

                                        TapHandler {
                                            onTapped: {
                                                root.queueCursor = row.index;
                                                Backend.queueJump(row.index);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        FadeText {
                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            visible: queueItems.count === 0
                            text: "Queue is empty"
                            color: Palette.onSurfaceVariant
                            font.pixelSize: Theme.fontSize("label", root.height)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                    }
                }
            }
        }

        // Context-aware keybar: the key hints; the album actions live on the
        // big cover (pane 3), not on chips here.
        Rectangle {
            id: keybar
            Layout.fillWidth: true
            Layout.preferredHeight: root.height * 0.055
            color: Palette.surfaceContainer
            radius: Theme.radius

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: parent.height * 0.4
                anchors.rightMargin: parent.height * 0.2
                spacing: root.width * 0.008

                FadeText {
                    Layout.fillWidth: true
                    Layout.alignment: Qt.AlignVCenter
                    text: root.keyHints()
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("label", root.height)
                    elide: Text.ElideRight
                    animated: false
                }

            }
        }
    }
}
