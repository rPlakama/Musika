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
import QtQuick.Effects
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Fullscreen player ("mainscreen"):
//   body   — lyrics own the space above the header;
//   header — identity/seek/transport anchored left, above the key hints;
//            the album name is the action surface and the star rating sits
//            directly under it;
//   footer — context-aware keybar for the focused pane.
// The play queue is a standalone drawer hosted by Home (Ctrl-Q).
// Esc (or q) leaves fullscreen; Space stays the global play/pause.
Rectangle {
    id: root
    radius: Theme.radius

    color: Palette.background

    /// Click/tap on the exit button (the header chip); Home leaves fullscreen.
    signal exitRequested
    /// ARC-UI-1: Alt-hold quick actions on the album surface; Home routes it
    /// to the shared overlay.
    signal quickAction(var target, var actions)

    // UI-57: the track title ramps with the window width like the list
    // headers do (ScrollPanel), toward the headline floor — never below it.
    readonly property real widthScale: Theme.widthScale(root.width)
    /// Star hitbox side under the album name (five slots total).
    readonly property real starUnit: Math.max(14, Math.min(30, root.height * 0.045))
    readonly property real trackTitlePixelSize: {
        var full = Theme.fontSize("headline", root.height);
        return Theme.clamp(full * root.widthScale, 18, full);
    }
    // Keyboard pane: 1 = lyrics, 3 = album actions. Transport is never a pane.
    property int pane: 1
    property bool albumStarred: false
    // UI-60: the playing album's user rating (0..=5, 0 = unrated).
    property int albumRating: 0
    property bool albumCacheOn: false

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
    // Keep the 50/50 body while the fetch is in flight; collapse only once
    // the track is known to have no lyrics.
    readonly property bool lyricsVisible: root.lyricsAvailable || root.lyricsLoading
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
    readonly property real lyricSize: Theme.clamp(lyricsPane.height * 0.055, 13, 22)
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
            return "j/k scroll   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab album   ·   Esc/Q exit";
        if (pane === 3)
            return "Enter album actions" + (lyricsVisible ? "   ·   Tab lyrics" : "") + "   ·   Esc exit";
        return "j/k scroll   ·   ,/. seek   ·   Ctrl-N/P track   ·   Tab album   ·   Esc/Q exit";
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

    /// The album name owns the album verbs (favourite + count + download).
    function coverActions() {
        return verbs.album(root.albumItem(), root.albumCacheOn, function (level) {
            root.albumRating = level;
        });
    }

    function scrollLyrics(delta) {
        root.holdLyrics();
        var view = lyricsView.view;
        // Origin-aware range (a header/footer would move `originY`); the pane
        // has none, so this is the plain content range.
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
        // A new song carries no cursor: clear it, then bring the opening line
        // into view — with the clamped follow row 0 pins to the top — and let
        // the first `followLyric` glide take over once the active phrase moves.
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

    // When a fetch settles with no lyrics, pane 1 no longer exists.
    onLyricsVisibleChanged: {
        if (!root.lyricsVisible && root.pane === 1)
            root.pane = 3;
    }

    // LYR-8: re-arm the idle window when pane focus moves (5 s vs 2 s).
    onPaneChanged: if (root._lyricsHeld)
        root.holdLyrics()

    /// ARC-UI-1: Alt armed the gesture — with the album pane focused, the
    /// album name owns the album verbs.
    function armQuickActions() {
        // UI-60: with the album pane focused, Alt anchors on the album name;
        // its verbs carry the favourite + count + download.
        if (root.pane === 3 && Backend.currentAlbumId.length > 0)
            root.quickAction(albumTitleBox, root.coverActions());
    }

    /// Tab walks the surfaces that exist: lyrics is skipped while hidden.
    function cyclePane(forward) {
        var order = root.lyricsVisible ? [1, 3] : [3];
        var i = order.indexOf(root.pane);
        if (i < 0)
            i = 0;
        else
            i = (i + (forward ? 1 : order.length - 1)) % order.length;
        root.pane = order[i];
    }

    // Fullscreen keys, scoped to the focused surface. Esc/back stays with Home.
    function handleAction(action) {
        if (action === "next_track") {
            Backend.playerNextBy(Math.max(1, Backend.keyCount));
            return;
        }
        if (action === "prev_track") {
            Backend.playerPreviousBy(Math.max(1, Backend.keyCount));
            return;
        }

        if (action === "pane_cycle" || action === "pane_cycle_back") {
            root.cyclePane(action === "pane_cycle");
            return;
        }

        if (pane === 3) {
            // The album name is the action surface: Enter arms its album
            // verbs (favourite, count, download) as a sticky menu.
            if (action === "enter" && Backend.currentAlbumId.length > 0) {
                var coverMenu = root.coverActions();
                coverMenu.sticky = true;
                root.quickAction(albumTitleBox, coverMenu);
            } else if ((action === "move_left" || action === "move_right") && root.lyricsVisible) {
                root.pane = 1;
            }
            return;
        }

        // VI-7: the count scales the lyric scroll.
        var lyricSteps = Math.max(1, Backend.keyCount);
        if (action === "move_up")
            scrollLyrics(-lyricsView.height * 0.3 * lyricSteps);
        else if (action === "move_down")
            scrollLyrics(lyricsView.height * 0.3 * lyricSteps);
        else if (action === "page_up")
            scrollLyrics(-lyricsView.height * 0.5);
        else if (action === "page_down")
            scrollLyrics(lyricsView.height * 0.5);
        else if (action === "move_left" || action === "move_right")
            root.pane = 3;
    }

    onVisibleChanged: {
        if (visible) {
            // UI-54: re-entering restores the last fullscreen scope (pane).
            // Only lyrics/album are panes — the queue is Home's drawer.
            Backend.scrollHydrate();
            var raw = Backend.scrollEntry("fullscreen");
            var saved = raw ? JSON.parse(raw) : null;
            pane = saved && (saved.scope === 1 || saved.scope === 3) ? saved.scope : 1;
            // Lyrics stay alive while the player is closed: fetch only when
            // the track changed behind the curtain, otherwise the lines stay
            // exactly where they were (no reload, no flick).
            if (root._lyricsSongId !== Backend.currentSongId) {
                lyricsLoading = true;
                Backend.requestLyrics();
            }
            if (root.pane === 1 && !root.lyricsVisible)
                root.pane = 3;
            // The keybar's album actions reconcile on entry too.
            root.requestAlbumActions();
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "lyrics" && root.visible)
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

    // ARC-UI-1: the recycled Alt verb set.
    QuickActions {
        id: verbs
    }

    ListModel {
        id: lyricModel
    }

    // D-5 ambience: the playing cover painted sharp over the shared blurred,
    // darkened backdrop. Hidden without art; the surfaces stay opaque.
    // UI-55: the panes sample exactly what is painted behind them.
    Item {
        id: ambience
        anchors.fill: parent

        CoverBackdrop {
            id: coverBackdrop
            anchors.fill: parent
            source: Backend.currentCoverUrl
        }
    }

    Item {
        anchors.fill: parent
        anchors.margins: root.width * 0.025

        // Header: identity, controls and transport anchored to the left,
        // sitting just above the key hints.
        RowLayout {
            id: header
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: keybar.top
            anchors.bottomMargin: root.height * 0.018
            spacing: root.width * 0.012

            ColumnLayout {
                id: infoHalf
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignLeft | Qt.AlignVCenter
                spacing: root.height * 0.008

                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.width * 0.008

                    FadeText {
                        Layout.fillWidth: true
                        text: Backend.currentTitle.length > 0 ? Backend.currentTitle : "No track playing"
                        color: Palette.onSurface
                        font.pixelSize: root.trackTitlePixelSize
                        font.bold: true
                        elide: Text.ElideRight
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

                // UI-60: the album name is the album action surface now that
                // the hero cover is gone; the rating sits directly under it.
                FadeText {
                    id: albumTitleBox
                    Layout.fillWidth: true
                    text: Backend.currentAlbum
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.fontSize("body", root.height)
                    elide: Text.ElideRight
                    visible: text.length > 0

                    ScopeRing {
                        anchors.fill: parent
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
                            root.quickAction(albumTitleBox, menu);
                        }
                    }
                }

                StarRating {
                    Layout.alignment: Qt.AlignLeft
                    unit: root.starUnit
                    rating: root.albumRating
                    onPicked: level => {
                        root.albumRating = level;
                        if (Backend.currentAlbumId.length > 0)
                            Backend.rateAlbum(Backend.currentAlbumId, level);
                    }
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

                    ScrubBar {
                        Layout.fillWidth: true
                        Layout.preferredHeight: Math.max(8, root.height * 0.02)
                        trackHeight: Math.max(2, root.height * 0.006)
                        value: Backend.duration > 0
                            ? Math.min(1, Backend.position / Backend.duration) : 0
                        onMoved: Backend.playerSeek(value * Backend.duration)
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
                    Layout.preferredHeight: Math.max(32, root.height * 0.058)
                    color: "transparent"
                    radius: Theme.radius

                    // Volume cluster (P-7): tiered icon (click = mute) + bar.
                    // The bar is the elastic part; the box clamps to whatever
                    // the transport is not using.
                    Rectangle {
                        id: volumeBox
                        radius: Theme.radius
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: Math.max(root.height * 0.04, Math.min(volumeRow.implicitWidth, transportBar.width - transportBox.width - root.width * 0.012))
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

                            ScrubBar {
                                visible: root.width >= Theme.breakpoints.narrow
                                Layout.fillWidth: true
                                Layout.minimumWidth: root.height * 0.02
                                Layout.maximumWidth: root.width * 0.06
                                Layout.preferredWidth: root.width * 0.06
                                Layout.preferredHeight: Math.max(8, root.height * 0.02)
                                trackHeight: Math.max(2, root.height * 0.006)
                                dimmed: Backend.muted
                                value: Math.max(0, Math.min(1, Backend.volume / 100))
                                onMoved: Backend.playerSetVolume(value * 100)
                            }
                        }
                    }

                    // Transport cluster (skip / pause / next) in its own
                    // rectangle: centred in the whole bar while there is room,
                    // and pushed left only as far as needed once the volume
                    // box would touch it — the transport yields, the volume
                    // stays put.
                    Rectangle {
                        id: transportBox
                        radius: Theme.radius
                        anchors.verticalCenter: parent.verticalCenter
                        width: transportRow.implicitWidth
                        height: parent.height
                        color: "transparent"
                        x: Math.max(0, Math.min((transportBar.width - width) / 2, transportBar.width - volumeBox.width - root.width * 0.012 - width))

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
        }

        // Body: the lyrics own the space above the header.
        RowLayout {
            id: body
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: header.top
            anchors.bottomMargin: root.height * 0.018
            spacing: root.width * 0.012

            Item {
                id: lyricsPane
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: root.lyricsVisible

                TapHandler {
                    onTapped: root.pane = 1
                }

                ColumnLayout {
                    anchors.fill: parent
                    spacing: root.height * 0.006

                    // The stage: an inner rectangle inside the pane that
                    // contains every lyric line, the scaled active one
                    // included. Delegates pre-calculate their emphasis so
                    // the grown line never reaches these walls; `clip` is
                    // the last-resort containment.
                    Item {
                        id: lyricsStageHost
                        Layout.fillWidth: true
                        Layout.fillHeight: true

                    Rectangle {
                        id: lyricsStage
                        anchors.fill: parent
                        radius: Theme.radius
                        color: "transparent"
                        clip: true
                        layer.enabled: true
                        layer.textureSize: Qt.size(
                            Math.max(1, Math.ceil(width * Screen.devicePixelRatio)),
                            Math.max(1, Math.ceil(height * Screen.devicePixelRatio)))
                        layer.effect: MultiEffect {
                            maskEnabled: true
                            maskSource: lyricsMaskTexture
                            maskThresholdMin: 0.0
                            maskThresholdMax: 1.0
                            maskSpreadAtMin: 0.0
                            maskSpreadAtMax: 0.0
                        }

                        ScrollContent {
                            id: lyricsView
                            anchors.fill: parent
                            context: "lyrics"
                            visible: root.lyricsAvailable
                            model: lyricModel
                            rowSpacing: root.lyricSize * 0.9
                            // Centred follow, clamped at the pane ends:
                            // the opening and closing lines pin to the
                            // edge instead of leaving an empty gap.
                            // LYR-8: the soothing lyrics follow — the
                            // shared glide at 300 ms that centres the
                            // phrase with the OutBack spring. The pane's
                            // fade rides the same value.
                            followMs: 300
                            // A width change re-wraps the lyric rows;
                            // let the shared resize repair recentre the
                            // active line, but never yank a manual scroll.
                            recenterOnWidth: root.visible && root.lyricsSynced && root.activeLyric >= 0 && !root._lyricsHeld && lyricsView.currentIndex === root.activeLyric

                            // A hand on the pane owns it until the next
                            // track.
                            Connections {
                                target: lyricsView.view
                                function onMovementStarted() {
                                    root.holdLyrics();
                                }
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
                                    // Reserve the emphasis growth on every
                                    // line, not only the active one: toggling
                                    // `active` must not change row heights (a
                                    // variable-height relayout on every phrase).
                                    // The maximum scale is the safe reservation;
                                    // the fit clamp can only make it smaller.
                                    readonly property real emphasisPad: Math.max(0, lyricLine.paintedHeight * (Theme.lyricsEmphasis - 1) / 2)

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
                                    scalable: true
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

                    ShaderEffectSource {
                        id: lyricsMaskTexture
                        sourceItem: lyricsMask
                        hideSource: true
                        live: true
                    }

                    Rectangle {
                        id: lyricsMask
                        anchors.fill: parent
                        readonly property real fade: 0.25
                        gradient: Gradient {
                            GradientStop { position: 0.0; color: "transparent" }
                            GradientStop { position: lyricsMask.fade; color: "white" }
                            GradientStop { position: 1.0 - lyricsMask.fade; color: "white" }
                            GradientStop { position: 1.0; color: "transparent" }
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
        }

        Rectangle {
            id: keybar
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: root.height * 0.055
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
