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
import "Theme.js" as Theme

// Album entry: hero cover beside the album description, then its songs.
// The art doubles as the ambience: a blurred copy fades into the theme
// surface behind the header (roadmap D-5).
//
// Set `albumId` to load the real track list from library.db; clicking a song
// queues the album at that track. The descriptive fields are still placeholders
// until album metadata is passed in.
Rectangle {
    radius: Theme.radius
    id: root

    signal back
    /// ARC-I-2: Alt-hold quick actions on the song row under the cursor;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    property string albumId: ""
    property bool onToolbar: false
    /// UI-60: the big cover is a selectable album surface (Enter/click arms
    /// its favourite + count + download verbs).
    property bool onCover: false
    /// Tab owns a local stop cycle (rows -> cover -> Back chip) instead of
    /// falling straight to the status bar.
    readonly property bool tabStops: true
    property string albumCover: ""
    property string albumTitle: ""
    property string albumArtist: ""
    property string albumMeta: ""
    /// ARC-I-2: the album's favourite state (header heart); fed by the
    /// opener from the album row and the ARC-43 page payload.
    property bool albumStarred: false
    /// UI-60: the album's user rating (0..=5, 0 = unrated); the
    /// `album_state` payload reconciles it on open.
    property int albumRating: 0
    /// CACHE-5/8: the album container's rule state (the cover's
    /// Download/Remove verb).
    property bool albumCacheOn: false
    property string albumNotes: ""
    // UI-14: album notes clamp to three lines until clicked.
    property bool notesExpanded: false
    // UI-22: true while the album's songs are in flight.
    property bool loading: false
    // UI-54: per-album scroll memory for the songs viewport.
    readonly property string memoryKey: "album:" + root.albumId
    property bool _restored: false

    // Header geometry (roadmap "Album page cover" D-3): the hero cover is
    // sized from the pane height, beside the description when there is room
    // and stacked over it on narrow panes.
    readonly property real headerHeight: Math.min(height * 0.44, 512 + height * 0.02)
    readonly property real coverSide: Math.max(160, Math.min(height * 0.42, 512))
    /// Star hitbox side under the cover (five slots total).
    readonly property real starUnit: Math.max(14, Math.min(28, root.coverSide * 0.075))
    readonly property bool sideBySide: width >= Theme.breakpoints.compact

    // The compact flip reflows the header: restart the cover's dip-and-blur
    // (below) so the reflow does not read as a flick.
    onSideBySideChanged: {
        if (!Theme.reduceMotion)
            coverAppear.restart();
    }

    color: Palette.surface

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }

    ListModel {
        id: songs
    }

    // ARC-I-2: the recycled Alt verb set; album songs are tracks.
    QuickActions {
        id: verbs
    }

    /// VI-4: the selection state lives in `ScrollContent` below; this page
    /// owns only its cursor/toolbar semantics.
    readonly property bool hasSelection: content.hasSelection
    readonly property int selectionCount: content.selectionCount

    /// Ask the backend for the download state of that window only.
    function requestMarkWindow(force) {
        var ids = [];
        var at = Math.max(0, content.currentIndex);
        var lo = Math.max(0, at - 15);
        var hi = Math.min(songs.count - 1, at + 15);
        for (var i = lo; i <= hi; ++i) {
            var row = songs.get(i);
            if (row && row.songId)
                ids.push(row.songId);
        }
        if (force)
            Backend.refreshDownloads(JSON.stringify(ids), "[]");
        else
            Backend.requestDownloads(JSON.stringify(ids), "[]");
    }

    /// VI-4: `v` toggles visual mode on the cursor row (the toolbar owns no
    /// selection).
    function toggleSelection() {
        if (root.onToolbar)
            return;
        content.toggleSelection();
    }

    /// VI-4: `V` selects the whole track list.
    function selectAll() {
        if (songs.count === 0)
            return;
        root.onToolbar = false;
        content.currentIndex = songs.count - 1;
        content.selectAllRows();
        content.goto();
    }

    function clearSelection() {
        content.clearSelection();
    }

    function albumItem() {
        return {
            albumId: root.albumId,
            starred: root.albumStarred,
            rating: root.albumRating
        };
    }

    // UI-60: the album cover's verbs — the same favourite + count + download
    // set the album rows use.
    function coverActions() {
        return verbs.album(root.albumItem(), root.albumCacheOn, function(level) {
            root.albumRating = level;
        });
    }

    function armCoverActions(sticky) {
        if (root.albumId.length === 0)
            return;
        var actions = root.coverActions();
        actions.sticky = sticky === true;
        root.quickAction(coverBox, actions);
    }

    /// ARC-I-2: Alt armed the gesture — on the cover it arms the album verbs;
    /// otherwise anchor on the song row under the cursor (the toolbar is a
    /// no-op, like an empty list). A visual selection swaps in the batch verb
    /// set (VI-4).
    function armQuickActions() {
        if (root.onCover) {
            root.armCoverActions(false);
            return;
        }
        if (root.onToolbar || songs.count === 0 || content.currentIndex < 0)
            return;
        var row = content.view.itemAtIndex(content.currentIndex);
        var actions = root.hasSelection
            ? verbs.forSelection("song", content.selectionIds("songId"),
                content.selectionStarred("songId"))
            : (songs.get(content.currentIndex) ? verbs.song(songs.get(content.currentIndex)) : null);
        if (!actions)
            return;
        // ARC-I-4: the heart tracks the verb immediately (see `MediaList`).
        if (actions.south && actions.south.invoke) {
            var want = actions.south.favorite === true;
            var star = actions.south.invoke;
            actions.south.invoke = function() {
                star();
                content.markStarred(want);
            };
        }
        if (row)
            quickAction(row, actions);
    }

    function reload() {
        if (root.albumId === "")
            return;
        root.loading = true;
        Backend.requestAlbumSongs(root.albumId);
        Backend.requestAlbumInfo(root.albumId);
        // UI-60: the header's favourite + rating marks reconcile here (the
        // opener's row flags may be stale); the cover's Download/Remove verb
        // reads the container rule state.
        Backend.requestAlbumState(root.albumId);
        Backend.requestAlbumCache(root.albumId);
    }

    function applyInfo(payload) {
        var info = JSON.parse(payload);
        root.albumNotes = info.notes ? info.notes : "";
    }

    function apply(payload) {
        // VI-4: a reload invalidates the selected range.
        content.clearSelection();
        songs.clear();
        var list = JSON.parse(payload);
        for (var i = 0; i < list.length; i++) {
            var song = list[i];
            songs.append({
                track: song.track ? String(song.track) : String(i + 1),
                title: song.title,
                duration: Theme.formatTime(song.duration),
                starred: song.starred === true,
                songId: song.id,
                albumId: song.album_id ? song.album_id : ""
            });
        }
        root.loading = false;
        // UI-54: reopening the page restores the cursor + offset; later
        // reloads (library change, resync) keep the live state.
        if (!root._restored) {
            root._restored = true;
            // The playing album opens on the playing track (the page is a
            // performance surface for it); otherwise a remembered cursor
            // lands via `restore()` and a fresh page lands on row 0
            // immediately — otherwise the first `j` only reaches row 0 and a
            // second press is needed to move (the "second J" bug).
            if (!root.followCurrentTrack(false) && !content.restore() && songs.count > 0)
                content.land(0);
        }
        root.requestMarkWindow();
    }

    /// Place the cursor on the playing track when this page's album is the
    /// one playing (`currentAlbumId` matches and the row's id is the playing
    /// song). `glide` = the shared centre glide (a live advance) versus the
    /// instant landing (page open). Returns true when the page owns the
    /// playing track.
    function followCurrentTrack(glide) {
        if (Backend.currentAlbumId.length === 0 || Backend.currentAlbumId !== root.albumId)
            return false;
        for (var i = 0; i < songs.count; ++i) {
            if (songs.get(i).songId !== Backend.currentSongId)
                continue;
            if (content.currentIndex !== i) {
                if (glide)
                    content.goto(i);
                else
                    content.land(i);
            }
            return true;
        }
        return false;
    }

    Component.onCompleted: reload()

    // The mark window follows the song cursor; the cursor follows the
    // playing track while this album is the one playing.
    Connections {
        target: content
        function onCurrentIndexChanged() { root.requestMarkWindow(); }
    }
    Connections {
        target: Backend
        function onCurrent_song_id_changed() { root.followCurrentTrack(true); }
    }

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`).
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow



    function searchRows() {
        return content.searchRows();
    }

    function searchJump(row) {
        content.searchJump(row);
    }

    function handleAction(action) {
        // UI-60: Tab cycles rows -> cover -> Back chip -> rows (Shift+Tab
        // reverses), so the cover action surface is keyboard-reachable.
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            var state = root.onCover ? 1 : (root.onToolbar ? 2 : 0);
            state = (state + (action === "pane_cycle" ? 1 : 2)) % 3;
            root.onCover = state === 1;
            root.onToolbar = state === 2;
            if (state !== 0)
                content.clearSelection();
            else if (songs.count > 0 && content.currentIndex < 0)
                content.currentIndex = 0;
            return;
        }
        // VI-11: `gg`/`G` leave the cover/toolbar for the first/last song;
        // the header stays a `k`-from-row-0 affordance only.
        if (action === "top" || action === "bottom") {
            if (songs.count === 0)
                return;
            root.onCover = false;
            root.onToolbar = false;
            content.edgeJump(action === "bottom");
            return;
        }
        if (action === "page_up" || action === "page_down") {
            if (songs.count === 0)
                return;
            root.onCover = false;
            root.onToolbar = false;
            content.pageJump(action === "page_down");
            return;
        }
        if (action.indexOf("extend_") === 0) {
            if (songs.count === 0)
                return;
            root.onCover = false;
            root.onToolbar = false;
            content.extendSelection("move_" + action.substring(7));
            return;
        }

        if (root.onCover) {
            // UI-60: the cover sits between the Back chip and the first row.
            if (action === "move_up") {
                root.onCover = false;
                root.onToolbar = true;
            } else if (action === "move_down" && songs.count > 0) {
                root.onCover = false;
                content.currentIndex = 0;
            } else if (action === "enter") {
                root.armCoverActions(true);
            }
            return;
        }

        if (root.onToolbar) {
            // Cursor is on the toolbar (Back). Down reaches the cover.
            if (action === "move_down") {
                root.onToolbar = false;
                root.onCover = true;
            } else if (action === "enter") {
                root.back();
            }
            return;
        }

        if (action === "move_up") {
            if (content.currentIndex > 0) {
                content.currentIndex -= 1;
            } else {
                // VI-4: leaving the rows for the cover ends the selection.
                content.clearSelection();
                root.onCover = true;
            }
        } else if (action === "move_down") {
            if (songs.count > 0)
                content.currentIndex = Math.min(songs.count - 1, content.currentIndex + 1);
        } else if (action === "enter") {
            content.clearSelection();
            Backend.playAlbum(root.albumId, content.currentIndex);
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "album_songs")
                root.apply(payload);
            else if (channel === "album_info")
                root.applyInfo(payload);
            else if (channel === "album_state") {
                var state = JSON.parse(payload);
                if (state.id === root.albumId) {
                    root.albumStarred = state.starred === true;
                    root.albumRating = Number(state.rating) || 0;
                }
            } else if (channel === "album_cache") {
                var cache = JSON.parse(payload);
                if (cache.id === root.albumId)
                    root.albumCacheOn = cache.on === true;
            }
        }
        // A fetch finished: re-read this page's own window (forced).
        function onRowReload() {
            root.requestMarkWindow(true);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }

    // Ambience (D-5): the album art itself through the shared
    // `CoverImage.ambient` mode — the GPU layer blurs and desaturates it, then
    // the gradient below washes it into the theme surface. This is the one
    // non-modal blur: content-derived ambience, never elevation (R2 amended).
    CoverImage {
        id: coverBackdrop
        anchors.fill: parent
        source: root.albumCover
        ambient: true
    }

    Rectangle {
        radius: Theme.radius
        anchors.fill: parent
        visible: coverBackdrop.visible
        gradient: Gradient {
            GradientStop {
                position: 0.0
                color: root.withAlpha(Palette.surface, 0.82)
            }
            GradientStop {
                position: 0.38
                color: root.withAlpha(Palette.surface, 0.30)
            }
            GradientStop {
                position: 1.0
                color: root.withAlpha(Palette.surface, 1.0)
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.02
        spacing: root.height * 0.02

        // Toolbar: back.
        RowLayout {
            Layout.fillWidth: true
            spacing: root.width * 0.02

            ExitChip {
                host: root
                label: "ESC/Q"
                highlighted: root.onToolbar
                onClicked: root.back()
            }

            Item {
                Layout.fillWidth: true
            }
        }

        // Header (D-3): cover hero + description. The grid collapses to one
        // column on narrow panes, which stacks the description under the
        // centered cover without a second layout.
        GridLayout {
            id: header
            Layout.fillWidth: true
            Layout.preferredHeight: root.sideBySide ? Math.max(root.headerHeight, root.coverSide + root.height * 0.01 + root.starUnit) : implicitHeight
            columns: root.sideBySide ? 2 : 1
            columnSpacing: root.width * 0.025
            rowSpacing: root.height * 0.012

            ColumnLayout {
                Layout.preferredWidth: root.coverSide
                Layout.preferredHeight: root.coverSide + root.height * 0.01 + root.starUnit
                Layout.alignment: root.sideBySide ? (Qt.AlignLeft | Qt.AlignVCenter) : Qt.AlignHCenter
                spacing: root.height * 0.01

                Rectangle {
                    id: coverBox
                    Layout.preferredWidth: root.coverSide
                    Layout.preferredHeight: root.coverSide
                    Layout.alignment: root.sideBySide ? (Qt.AlignLeft | Qt.AlignVCenter) : Qt.AlignHCenter
                    radius: Theme.radius
                    color: Palette.primaryContainer
                    clip: true

                    // The reflow FX: the offscreen layer only exists while the
                    // blur is up. `layer.effect` is a Component — nothing inside
                    // it can carry a reachable id — so the blur value lives on
                    // the cover and the effect binds to it.
                    property real heroBlur: 0
                    layer.enabled: heroBlur > 0.001
                    layer.effect: MultiEffect {
                        blurEnabled: true
                        blur: coverBox.heroBlur
                        blurMax: 48
                        autoPaddingEnabled: false
                    }

                    // Crossing the compact breakpoint reflows the hero: dim it to
                    // 70% and blur it in over 200 ms, then restore both over the
                    // emphasized tier — the reflow hides under the dip.
                    SequentialAnimation {
                        id: coverAppear
                        ParallelAnimation {
                            NumberAnimation {
                                target: coverBox
                                property: "opacity"
                                to: 0.7
                                duration: 200
                            }
                            NumberAnimation {
                                target: coverBox
                                property: "heroBlur"
                                to: 0.5
                                duration: 200
                            }
                        }
                        ParallelAnimation {
                            NumberAnimation {
                                target: coverBox
                                property: "opacity"
                                to: 1
                                duration: Theme.motion.emphasized
                                easing.type: Easing.Bezier
                                easing.bezierCurve: Theme.ease.decelerate
                            }
                            NumberAnimation {
                                target: coverBox
                                property: "heroBlur"
                                to: 0
                                duration: Theme.motion.emphasized
                                easing.type: Easing.Bezier
                                easing.bezierCurve: Theme.ease.decelerate
                            }
                        }
                    }

                    CoverImage {
                        id: coverImage
                        anchors.fill: parent
                        source: root.albumCover
                    }

                    // UI-60: the cover is a selectable album action surface.
                    ScopeRing {
                        shown: root.onCover
                    }

                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: {
                            root.onToolbar = false;
                            root.onCover = true;
                            root.armCoverActions(true);
                        }
                    }
                }

                StarRating {
                    Layout.alignment: Qt.AlignHCenter
                    unit: root.starUnit
                    rating: root.albumRating
                    onPicked: (level) => {
                        root.albumRating = level;
                        Backend.rateAlbum(root.albumId, level);
                    }
                }
            }

            // Description.
            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.006

                FadeText {
                    Layout.fillWidth: true
                    text: root.albumTitle
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.036
                    font.bold: true
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: root.albumArtist
                    color: Palette.primary
                    font.pixelSize: root.height * 0.024
                    elide: Text.ElideRight
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.height * 0.008

                    // ARC-I-2/UI-60: the album's favourite leads the header
                    // meta, with the rating count at its corner (a rated but
                    // unfavourited album still shows the badge).
                    Item {
                        visible: root.albumStarred || root.albumRating > 0
                        Layout.alignment: Qt.AlignVCenter
                        Layout.preferredWidth: root.height * 0.016
                        Layout.preferredHeight: width

                        Icon {
                            anchors.fill: parent
                            visible: root.albumStarred
                            name: "baseline-favorite"
                            symbolColor: Theme.iconStrong
                        }

                        RatingBadge {
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            width: parent.width * 0.952
                            height: width
                            rating: root.albumRating
                        }
                    }

                    FadeText {
                        Layout.fillWidth: true
                        text: root.albumMeta
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02
                        elide: Text.ElideRight
                    }
                }
                FadeText {
                    id: notesText
                    Layout.fillWidth: true
                    Layout.topMargin: root.height * 0.008
                    visible: root.albumNotes.length > 0
                    text: root.albumNotes
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                    lineHeight: 1.25
                    wrapMode: Text.WordWrap
                    // UI-14: clamped to three lines; click expands.
                    maximumLineCount: root.notesExpanded ? 1000 : 3
                    elide: root.notesExpanded ? Text.ElideNone : Text.ElideRight

                    HoverHandler {
                        enabled: root.albumNotes.length > 0
                        cursorShape: Qt.PointingHandCursor
                    }
                    TapHandler {
                        enabled: root.albumNotes.length > 0
                        onTapped: root.notesExpanded = !root.notesExpanded
                    }
                }
            }
        }

        Rectangle {
            radius: Theme.radius
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Palette.outlineVariant
        }

        // Tracks.
        RowLayout {
            Layout.fillWidth: true
            spacing: root.height * 0.012

            FadeText {
                text: "Tracks"
                color: Palette.primary
                font.pixelSize: root.height * 0.024
                font.bold: true
                font.letterSpacing: root.height * 0.004
            }

            Item {
                Layout.fillWidth: true
            }

            // VI-4: the selection count pill.
            SelectionBadge {
                Layout.alignment: Qt.AlignVCenter
                count: root.selectionCount
                pixelSize: root.height * 0.02
            }
        }

        ScrollContent {
            id: content
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: songs
            selectable: true
            memoryKey: root.memoryKey
            visible: !(root.loading && songs.count === 0)
            // ARC-UI-2 M1: the rows paint track, title and duration.
            searchFields: ["track", "title", "duration"]
            // UI-58: rows scale from the page height, not the song viewport
            // (the header/notes reflow must not rescale every row). The
            // precomputed extent keeps the shared glide off Qt's lazy
            // estimate.
            rowRatio: 0.055
            rowRefHeight: root.height

            rowDelegate: Component {
                Rectangle {
                    radius: Theme.radius
                    id: songRow
                    required property int index
                    required property string track
                    required property string title
                    required property string duration
                    required property string songId
                    required property string albumId
                    required property bool starred

                    // VI-4: a row inside the visual selection wears the cursor fill.
                    readonly property bool selectedRow: content.selected(index)
                    readonly property bool emphasized: ListView.isCurrentItem
                        || rowHover.hovered

                    width: ListView.view ? ListView.view.width : 0
                    // UI-58: the model's page-relative adaptive row.
                    height: content.adaptiveRowHeight
                    color: ((ListView.isCurrentItem && !root.onToolbar) || songRow.selectedRow)
                        ? Palette.secondaryContainer
                        : (rowHover.hovered
                            ? Palette.surfaceContainerHigh
                            : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest))

                    Behavior on color {
                        ColorAnimation {
                            duration: Theme.motion.quick
                            easing.type: Easing.Bezier
                            easing.bezierCurve: Theme.ease.standard
                        }
                    }

                    // The shared list row: AlbumEntry is the no-cover context.
                    RowBody {
                        anchors.fill: parent
                        viewWidth: songRow.width
                        rowHeight: songRow.height
                        emphasized: songRow.emphasized
                        showCover: false
                        showSubtitle: false
                        title: songRow.track.length > 0
                            ? songRow.track + ". " + songRow.title
                            : songRow.title
                        meta: songRow.duration
                        titlePixelSize: songRow.height * 0.34
                        metaPixelSize: songRow.height * 0.30
                        reserveMeta: true
                        metaAlignRight: true
                        leftMargin: songRow.width * 0.02
                        textGap: songRow.width * 0.02
                        rightMargin: songRow.width * 0.02
                        marksMargin: songRow.width * 0.01275
                        markCursor: content.currentIndex
                        markIndex: songRow.index
                        markSongId: songRow.songId
                        markAlbumId: songRow.albumId
                        markStarred: songRow.starred
                        markRatingVisible: false
                        searchQuery: content.searchQuery
                        searchCurrent: songRow.index === content.searchCurrentRow
                        expected: songRow.index !== content.currentIndex
                            && content.isExpected(songRow.y)
                        immediate: songRow.index === content.currentIndex
                    }

                    HoverHandler {
                        id: rowHover
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        acceptedModifiers: Qt.ShiftModifier
                        onTapped: {
                            root.onCover = false;
                            root.onToolbar = false;
                            content.extendSelectionTo(songRow.index);
                        }
                    }
                    TapHandler {
                        acceptedModifiers: Qt.NoModifier
                        onTapped: {
                            // VI-4: direct play ends the visual selection.
                            root.onCover = false;
                            root.onToolbar = false;
                            content.clearSelection();
                            content.currentIndex = songRow.index;
                            Backend.playAlbum(root.albumId, songRow.index);
                        }
                    }
                }
            }
        }

        // UI-22: the songs area has a wait state and a real empty-album state.
        SkeletonList {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: root.loading && songs.count === 0
        }

        FadeText {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !root.loading && songs.count === 0
            text: "No tracks in this album"
            color: Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("body", root.height)
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
    }

}
