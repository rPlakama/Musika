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
import "CoverSize.js" as CoverSize

// Album entry: the album art is the page ambience, with the identity and
// track list laid over it instead of a second hero cover (roadmap D-5).
//
// Set `albumId` to load the real track list from library.db; clicking a song
// queues the album at that track. The descriptive fields are still placeholders
// until album metadata is passed in.
Rectangle {
    id: root
    radius: Theme.radius

    RowMetrics {
        id: rowMetrics
        viewWidth: content.width
        rowHeight: content.adaptiveRowHeight
    }

    signal back
    /// ARC-I-2: Alt-hold quick actions on the song row under the cursor;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)
    /// Opens the album's owning artist page from the header name.
    signal artistActivated(string artistId, string artistName, string artistCover)

    /// "album" or "playlist": the same page anatomy, different source and art.
    property string entryKind: "album"
    property string albumId: ""
    property string playlistId: ""
    property string playlistTitle: ""
    readonly property bool isPlaylist: root.entryKind === "playlist"
    /// Distinct album-origin covers for the playlist collage.
    property var playlistCollage: []
    /// Search-hit song to park the cursor on once the rows land.
    property string focusSongId: ""
    property bool onToolbar: false
    /// UI-60: the big cover is a selectable album surface (Enter/click arms
    /// its favourite + count + download verbs).
    property bool onCover: false
    /// The header's artist name is its own Tab stop; Enter/click opens the
    /// owning artist's page.
    property bool onArtist: false
    /// UI-14: the description is its own Tab stop; selecting it expands the
    /// one-line notes into the scrollable reader.
    property bool onNotes: false
    /// Tab owns a local header stop cycle (cover -> artist -> notes -> Back
    /// chip) instead of falling straight to the status bar; the song rows are
    /// left alone for j/k.
    readonly property bool tabStops: true
    property string albumCover: ""
    property string albumTitle: ""
    property string albumArtist: ""
    property string playlistSubtitle: ""
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
    /// UI-14: one line by default; the Tab-selected description expands it
    /// into a scrollable reader (no line cap, no cut).
    property bool notesExpanded: false
    // UI-22: true while the album's songs are in flight.
    property bool loading: false
    // UI-54: per-album/per-playlist scroll memory for the songs viewport.
    readonly property string memoryKey: root.isPlaylist
        ? "playlist:" + root.playlistId : "album:" + root.albumId
    property bool _restored: false
    /// A session-restored page starts on row 0, like a scope or order reset.
    property bool resetCursorOnOpen: false
    property bool _focusApplied: false
    property int _coverTarget: 0
    // Album whose rows request is in flight; avoids reload storms from
    // duplicate open paths and library-changed notifications.
    property string _loadingAlbumId: ""

    /// Metadata stats merged after the album date:
    /// `<N> Tracks · <m:ss> · <AUDIO TAG>`.
    property int tracksDuration: 0
    property string tracksFormat: ""
    readonly property string tracksSummary: {
        var parts = [];
        var count = songs.count;
        if (count > 0)
            parts.push(Theme.countLabel(count, "Track"));
        if (root.tracksDuration > 0)
            parts.push(Theme.formatTime(root.tracksDuration));
        if (root.tracksFormat.length > 0)
            parts.push(root.tracksFormat);
        return Theme.metaJoin(parts);
    }
    /// The header metadata line: date first, then the track stats.
    readonly property string albumMetaLine: Theme.metaJoin([root.albumMeta, root.tracksSummary])

    /// Star hitbox side in the header (five slots total). 60% larger than
    /// the old strip, matching the album-title scale.
    readonly property real starUnit: Math.max(22, Math.min(45, root.height * 0.04))

    /// The header gap follows the cover but never eats more than 75% of
    /// the usable page height; wide pages keep room for the rows.
    readonly property real headerCoverBottom: Math.min(
        (root.isPlaylist ? playlistArt.coverSide : coverBackdrop.coverSide) - root.width * 0.02,
        (root.height - root.width * 0.04) * 0.75)

    color: Palette.surface

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
        return verbs.album(root.albumItem(), root.albumCacheOn, function (level) {
            root.albumRating = level;
        });
    }

    function armCoverActions(sticky) {
        if (root.isPlaylist || root.albumId.length === 0)
            return;
        var actions = root.coverActions();
        actions.sticky = sticky === true;
        root.quickAction(coverBox, actions);
    }

    /// ARC-I-2: Alt armed the gesture — on the cover it arms the album verbs;
    /// otherwise the shared body anchors on the song row under the cursor
    /// (the toolbar is a no-op, like an empty list).
    function armQuickActions() {
        if (root.onCover) {
            root.armCoverActions(false);
            return;
        }
        if (root.onToolbar)
            return;
        content.armQuickActions();
    }

    function scheduleCover() {
        if (root.isPlaylist || root.albumId === "")
            return;
        coverTimer.restart();
    }

    function requestCoverNow() {
        var target = CoverSize.target(coverBackdrop.coverSide, Screen.devicePixelRatio, Math.max(Screen.width, Screen.height));
        if (target <= 0) {
            root._coverTarget = 0;
            return;
        }
        if (target === root._coverTarget)
            return;
        root._coverTarget = target;
        Backend.requestAlbumCover(root.albumId, target);
    }

    function reload() {
        if (root.isPlaylist) {
            if (root.playlistId === "" || root._loadingAlbumId === root.playlistId)
                return;
            root._loadingAlbumId = root.playlistId;
            root.loading = true;
            root.onNotes = false;
            root.setNotesExpanded(false);
            Backend.requestPlaylist(root.playlistId);
            return;
        }
        if (root.albumId === "")
            return;
        // One request per album at a time. Startup can deliver a restored
        // page and a library-changed notification in the same turn; the
        // second must not bump the first payload away.
        if (root._loadingAlbumId === root.albumId)
            return;
        root._loadingAlbumId = root.albumId;
        root.loading = true;
        root.onNotes = false;
        root.setNotesExpanded(false);
        Backend.requestAlbumSongs(root.albumId);
        Backend.requestAlbumInfo(root.albumId);

        // The display copy is requested from the painted cover box (DPR +
        // screen-capped bucket); the original stays on screen until it lands.
        root._coverTarget = 0;
        root.scheduleCover();

        Backend.requestAlbumState(root.albumId);
        Backend.requestAlbumCache(root.albumId);
    }
    function applyInfo(payload) {
        var info = JSON.parse(payload);
        if (info.album_id && info.album_id !== root.albumId)
            return;
        root.albumNotes = info.notes ? info.notes : "";
        if (root.albumNotes.length === 0) {
            root.onNotes = false;
            root.setNotesExpanded(false);
        }
    }

    /// Resolve the album's owning artist and let Home push its page.
    function activateArtist() {
        if (root.isPlaylist || root.albumArtist.length === 0)
            return;
        Backend.requestAlbumArtist(root.albumId);
    }

    /// UI-14: expand/collapse the notes reader. The expanded block is a
    /// Flickable capped to the header's share of the pane; short notes show
    /// whole, long ones scroll without eliding.
    function setNotesExpanded(expanded) {
        root.notesExpanded = expanded === true;
        if (notesView)
            notesView.contentY = 0;
    }

    /// Clear every header Tab stop so the rows regain focus; the notes
    /// reader collapses too unless a caller is about to toggle it.
    function clearHeaderStops(collapseNotes) {
        root.onCover = false;
        root.onArtist = false;
        root.onNotes = false;
        root.onToolbar = false;
        if (collapseNotes !== false)
            root.setNotesExpanded(false);
    }

    function scrollNotes(delta) {
        if (!root.notesExpanded || !notesView)
            return;
        var max = Math.max(0, notesView.contentHeight - notesView.height);
        notesView.contentY = Math.max(0, Math.min(max, notesView.contentY + delta));
    }

    /// Park the cursor on the song a search hit asked for, once its row
    /// exists. Returns false when there is no focus or the song is not in
    /// this album (the normal restore / row-0 path then applies).
    function focusRequestedSong() {
        if (root._focusApplied || root.focusSongId.length === 0)
            return false;
        for (var i = 0; i < songs.count; ++i) {
            if (songs.get(i).songId !== root.focusSongId)
                continue;
            root._focusApplied = true;
            content.land(i);
            return true;
        }
        return false;
    }

    function applyPlaylist(payload) {
        var data = JSON.parse(payload);
        if (data.playlist_id !== root.playlistId)
            return;
        root._loadingAlbumId = "";
        root.albumTitle = data.name ? data.name : "";
        root.playlistSubtitle = data.subtitle ? data.subtitle : "";
        root.albumMeta = data.meta ? data.meta : "";
        root.playlistCollage = data.collage ? data.collage : [];
        root.tracksDuration = 0;
        root.tracksFormat = "";
        content.clearSelection();
        songs.clear();
        var rows = data.songs ? data.songs : [];
        for (var i = 0; i < rows.length; i++) {
            var row = rows[i];
            songs.append({
                track: row.track ? String(row.track) : String(i + 1),
                title: row.title,
                subtitle: row.subtitle ? String(row.subtitle) : "",
                duration: row.duration ? String(row.duration) : "",
                cover: row.cover ? String(row.cover) : "",
                songId: row.songId ? String(row.songId) : "",
                albumId: row.albumId ? String(row.albumId) : "",
                starred: row.starred === true,
                playable: row.playable === true
            });
        }
        root.loading = false;
        root._restored = true;
        if (songs.count > 0)
            content.land(0);
    }

    function apply(payload) {
        var data = JSON.parse(payload);
        // The album history can have more than one page alive; never let a
        // payload for another album clear this page.
        if (data.album_id !== root.albumId)
            return;
        var list = data.songs ? data.songs : [];
        // VI-4: a reload invalidates the selected range.
        content.clearSelection();
        songs.clear();
        root._loadingAlbumId = "";
        // Tracks-header stats: total runtime + the most common file suffix.
        var total = 0;
        var formats = {};
        for (var i = 0; i < list.length; i++) {
            var song = list[i];
            total += Number(song.duration) || 0;
            var suffix = song.suffix ? String(song.suffix).toLowerCase() : "";
            if (suffix.charAt(0) === ".")
                suffix = suffix.substring(1);
            if (suffix.length > 0)
                formats[suffix] = (formats[suffix] || 0) + 1;
            songs.append({
                track: song.track ? String(song.track) : String(i + 1),
                title: song.title,
                subtitle: song.artist ? String(song.artist) : "",
                duration: song.meta,
                starred: song.starred === true,
                songId: song.id,
                albumId: song.albumId ? song.albumId : "",
                cover: "",
                playable: true
            });
        }
        root.tracksDuration = total;
        var format = "";
        var formatCount = 0;
        for (var key in formats) {
            if (formats[key] > formatCount) {
                format = key;
                formatCount = formats[key];
            }
        }
        root.tracksFormat = format.toUpperCase();
        root.loading = false;
        // Session return resets the page cursor, matching a scope/order reset.
        if (root.resetCursorOnOpen) {
            root._restored = true;
            if (songs.count > 0)
                content.land(0);
            return;
        }
        // A search jump owns the first cursor; otherwise UI-54 reopens on the
        // remembered cursor and later reloads keep the live state.
        if (root.focusRequestedSong()) {
            root._restored = true;
        } else if (!root._restored) {
            root._restored = true;
            // The playing album opens on the playing track (the page is a
            // performance surface for it); otherwise a remembered cursor
            // lands via `restore()` and a fresh page lands on row 0
            // immediately — otherwise the first `j` only reaches row 0 and a
            // second press is needed to move (the "second J" bug).
            if (!root.followCurrentTrack(false) && !content.restore() && songs.count > 0)
                content.land(0);
        }
    }

    /// Place the cursor on the playing track when this page's album is the
    /// one playing (`currentAlbumId` matches and the row's id is the playing
    /// song). `glide` = the shared centre glide (a live advance) versus the
    /// instant landing (page open). Returns true when the page owns the
    /// playing track.
    function followCurrentTrack(glide) {
        if (root.isPlaylist)
            return false;
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

    Timer {
        id: coverTimer
        interval: 120
        repeat: false
        onTriggered: root.requestCoverNow()
    }

    onPlaylistIdChanged: if (root.isPlaylist) Qt.callLater(root.reload)
    onEntryKindChanged: if (root.isPlaylist) Qt.callLater(root.reload)
    Component.onCompleted: reload()

    // The cursor follows the playing track while this album is the one
    // playing.
    Connections {
        target: Screen
        function onDevicePixelRatioChanged() {
            root._coverTarget = 0;
            root.scheduleCover();
        }
    }
    Connections {
        target: Backend
        function onCurrent_song_id_changed() {
            root.followCurrentTrack(true);
        }
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

    /// VI-8a: yank the cursor song row (`Y`) or the visual range (`y`).
    function yankRows(rowOnly) {
        if (songs.count === 0 || content.currentIndex < 0)
            return;
        var indices = (rowOnly || !root.hasSelection) ? [content.currentIndex] : content.selectionIndices();
        var ids = [];
        for (var i = 0; i < indices.length; ++i) {
            var item = songs.get(indices[i]);
            if (item && item.songId)
                ids.push(item.songId);
        }
        if (ids.length === 0)
            return;
        content.clearSelection();
        Backend.yankRows("song", JSON.stringify(ids));
    }

    function playRow(index) {
        if (index < 0 || index >= songs.count)
            return;
        if (root.isPlaylist) {
            var row = songs.get(index);
            if (row && row.playable === true)
                Backend.playPlaylist(root.playlistId, index);
            return;
        }
        Backend.playAlbum(root.albumId, index);
    }

    function handleAction(action) {
        // UI-60/UI-14: Tab walks the header only: cover -> artist -> notes ->
        // Back chip (Shift+Tab reverses), leaving the song rows to j/k. Empty
        // artist/notes stops are skipped; selecting the notes expands the
        // reader.
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            var state = root.onCover ? 1 : (root.onArtist ? 2 : (root.onNotes ? 3 : (root.onToolbar ? 4 : 0)));
            do {
                state = (state + (action === "pane_cycle" ? 1 : 4)) % 5;
            } while ((state === 2 && root.albumArtist.length === 0) || (state === 3 && root.albumNotes.length === 0))
            root.onCover = state === 1;
            root.onArtist = state === 2;
            root.onNotes = state === 3;
            root.onToolbar = state === 4;
            root.setNotesExpanded(state === 3);
            if (state !== 0)
                content.clearSelection();
            else if (songs.count > 0 && content.currentIndex < 0)
                content.currentIndex = 0;
            return;
        }
        // VI-11: `gg`/`G` leave the header for the first/last song;
        // the header stays a `k`-from-row-0 affordance only.
        if (action === "top" || action === "bottom") {
            if (songs.count === 0)
                return;
            root.clearHeaderStops();
            content.edgeJump(action === "bottom");
            return;
        }
        if (action === "page_up" || action === "page_down") {
            if (root.onNotes && root.notesExpanded) {
                root.scrollNotes(action === "page_up" ? -notesView.height * 0.9 : notesView.height * 0.9);
                return;
            }
            if (songs.count === 0)
                return;
            root.clearHeaderStops();
            content.pageJump(action === "page_down");
            return;
        }
        if (action.indexOf("extend_") === 0) {
            if (songs.count === 0)
                return;
            root.clearHeaderStops();
            content.extendSelection("move_" + action.substring(7));
            return;
        }

        if (root.onArtist) {
            // The header's artist name: Enter opens the owning artist page.
            if (action === "enter") {
                root.activateArtist();
            } else if (action === "move_up") {
                root.onArtist = false;
                root.onCover = true;
            } else if (action === "move_down") {
                root.onArtist = false;
                if (songs.count > 0)
                    content.currentIndex = 0;
            }
            return;
        }

        if (root.onNotes) {
            // UI-14: an expanded reader owns j/k and page keys for scrolling;
            // Enter collapses it. When collapsed, up/down leave the stop.
            if (action === "move_up" || action === "move_down") {
                if (root.notesExpanded) {
                    var line = Math.max(12, notesText.font.pixelSize * notesText.lineHeight);
                    root.scrollNotes(action === "move_up" ? -line : line);
                } else if (action === "move_up") {
                    root.onNotes = false;
                    if (root.albumArtist.length > 0)
                        root.onArtist = true;
                    else
                        root.onCover = true;
                } else {
                    root.onNotes = false;
                    root.onToolbar = true;
                }
            } else if (action === "enter") {
                root.setNotesExpanded(!root.notesExpanded);
            }
            return;
        }

        if (root.onCover) {
            // UI-60: the cover sits between the Back chip and the artist link.
            if (action === "move_up") {
                root.onCover = false;
                root.onToolbar = true;
            } else if (action === "move_down") {
                root.onCover = false;
                if (root.albumArtist.length > 0)
                    root.onArtist = true;
                else if (songs.count > 0)
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

        // VI-8a: yank the song rows into the universal clipboard.
        if (action === "yank" || action === "yank_row") {
            root.yankRows(action === "yank_row");
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
            if (channel === "playlist") {
                root.applyPlaylist(payload);
            } else if (channel === "album_songs")
                root.apply(payload);
            else if (channel === "album_info")
                root.applyInfo(payload);
            else if (channel === "album_cover") {
                var art = JSON.parse(payload);
                // A slower request for an older width bucket must not replace
                // the copy requested for the current cover box. Never swap a
                // display copy in after the original has painted: the second
                // decode blanks the cover (and on session return the first
                // cover is already up by the time this cached copy arrives).
                // The copy is still cached for the next open.
                if (art.album_id === root.albumId && art.cover && Number(art.size) === root._coverTarget && !coverBackdrop.ready)
                    root.albumCover = art.cover;
            } else if (channel === "album_artist") {
                var link = JSON.parse(payload);
                if (link.album_id === root.albumId && link.artist_id)
                    root.artistActivated(link.artist_id, root.albumArtist, root.albumCover);
            } else if (channel === "album_state") {
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
        function onLibrary_changed() {
            root.reload();
        }
    }

    // Ambience (D-5): the album art itself, painted sharp with no wash
    CoverBackdrop {
        id: coverBackdrop
        anchors.fill: parent
        visible: !root.isPlaylist
        source: root.albumCover
        // Leave the bottom quarter for the rows; the square never grows
        // behind them.
        maxSide: (root.height - root.width * 0.04) * 0.75
        onCoverSideChanged: root.scheduleCover()
    }

    // Playlist mode: the same square ambience, split by distinct album origins.
    PlaylistCollage {
        id: playlistArt
        anchors.fill: parent
        visible: root.isPlaylist
        sources: root.playlistCollage
        maxSide: (root.height - root.width * 0.04) * 0.75
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.02
        // The track scroller owns the page bottom edge; the side and top
        // gutters stay inset.
        anchors.bottomMargin: 0
        spacing: root.height * 0.02

        // Header: the album art is only the page ambience now; the identity
        // gets the full width and stays anchored at the top of the page.
        ColumnLayout {
            id: header
            Layout.fillWidth: true
            // The rows start below the displayed square art, not on top of it.
            Layout.bottomMargin: Math.max(0, root.headerCoverBottom - header.implicitHeight - (headerGap.visible ? root.height * 0.22 : 0))
            spacing: root.height * 0.008

            // The identity sits on the artwork, so give it a 40% dark
            // backdrop; alpha is on the fill so the text keeps full opacity.
            Item {
                id: headerPanelWrap
                // The panel hugs the longest identity line, capped by the page
                // width; alpha is on the fill so the text keeps full opacity.
                Layout.alignment: Qt.AlignLeft
                Layout.fillWidth: false
                implicitWidth: Math.min(headerContent.implicitWidth + root.width * 0.02, root.width * 0.96)
                implicitHeight: headerContent.implicitHeight + root.width * 0.02
                Layout.preferredWidth: implicitWidth
                Layout.maximumWidth: root.width * 0.96
                Layout.preferredHeight: implicitHeight

                // A separate blurred silhouette behind the panel; a layer shadow
                // on the panel itself clips at the panel edge and reads sharp.
                Rectangle {
                    id: headerShadow
                    x: root.width * 0.008
                    y: root.width * 0.01
                    width: parent.width
                    height: parent.height
                    radius: Theme.radius
                    color: Qt.rgba(0, 0, 0, 0.55)
                    opacity: 0.35
                    layer.enabled: true
                    layer.effect: MultiEffect {
                        blurEnabled: true
                        blur: 1.0
                        blurMax: 64
                        autoPaddingEnabled: true
                    }
                }

                Rectangle {
                    id: headerPanel
                    anchors.fill: parent
                    color: Qt.rgba(0, 0, 0, 0.4)
                    radius: Theme.radius

                    ColumnLayout {
                    id: headerContent
                    anchors.fill: parent
                    anchors.margins: root.width * 0.01
                    spacing: root.height * 0.006

            // Description.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: root.height * 0.006

                // UI-60: the title is the album action surface now that the
                // hero cover only lives in the background.
                FadeText {
                    id: coverBox
                    Layout.fillWidth: true
                    text: root.albumTitle
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.04
                    font.bold: true
                    elide: Text.ElideRight

                    ScopeRing {
                        anchors.fill: parent
                        shown: root.onCover
                    }

                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: {
                            root.clearHeaderStops();
                            root.onCover = true;
                            root.armCoverActions(true);
                        }
                    }
                }

                FadeText {
                    Layout.fillWidth: true
                    visible: root.isPlaylist
                    text: root.playlistSubtitle
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.024
                    elide: Text.ElideRight
                }

                StarRating {
                    visible: !root.isPlaylist
                    Layout.alignment: Qt.AlignLeft
                    unit: root.starUnit
                    rating: root.albumRating
                    onPicked: level => {
                        root.albumRating = level;
                        Backend.rateAlbum(root.albumId, level);
                    }
                }

                FadeText {
                    id: artistLink
                    visible: !root.isPlaylist
                    Layout.fillWidth: true
                    text: root.albumArtist
                    color: Palette.primary
                    font.pixelSize: root.height * 0.024
                    elide: Text.ElideRight

                    ScopeRing {
                        anchors.fill: parent
                        shown: root.onArtist
                    }

                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }

                    TapHandler {
                        onTapped: {
                            root.clearHeaderStops(false);
                            root.onArtist = true;
                            root.activateArtist();
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.height * 0.008

                    // ARC-I-2/UI-60: the album's favourite leads the header
                    // meta (the star row above owns the rating now).
                    Item {
                        visible: !root.isPlaylist && root.albumStarred
                        Layout.alignment: Qt.AlignVCenter
                        Layout.preferredWidth: root.height * 0.016
                        Layout.preferredHeight: width

                        Icon {
                            anchors.fill: parent
                            name: "baseline-favorite"
                            symbolColor: Theme.iconStrong
                        }
                    }

                    FadeText {
                        Layout.fillWidth: true
                        text: root.albumMetaLine
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02
                        elide: Text.ElideRight
                    }

                    // VI-4: the selection count pill follows the metadata.
                    SelectionBadge {
                        Layout.alignment: Qt.AlignVCenter
                        count: root.selectionCount
                        pixelSize: root.height * 0.02
                    }
                }
            }
                }
            }
        }
        }

        // The header's bottom gap: an empty band (or the description when
        // there is one) between the identity at the top and the Tracks list
        // at the bottom. The gap only grows downward — the header itself
        // never stretches.
        Item {
            id: headerGap
            Layout.fillWidth: true
            Layout.fillHeight: false
            Layout.preferredHeight: notesBox.visible ? root.height * 0.22 : 0
            visible: notesBox.visible
            clip: true

            // UI-14: the description fills the top of the gap. Text wraps
            // inside it; long notes scroll (wheel/trackpad or j/k while the
            // notes stop is selected) instead of eliding.
            Item {
                id: notesBox
                anchors.top: parent.top
                anchors.left: parent.left
                anchors.right: parent.right
                height: root.albumNotes.length > 0 ? Math.min(parent.height, root.height * 0.325) : 0
                visible: !root.isPlaylist && root.albumNotes.length > 0
                clip: true

                Flickable {
                    id: notesView
                    anchors.fill: parent
                    contentWidth: width
                    contentHeight: notesText.implicitHeight
                    boundsBehavior: Flickable.StopAtBounds
                    interactive: contentHeight > height
                    clip: true

                    FadeText {
                        id: notesText
                        width: notesView.width
                        text: root.albumNotes
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02
                        lineHeight: 1.25
                        wrapMode: Text.WordWrap
                        maximumLineCount: 0
                        elide: Text.ElideNone
                    }
                }

                ScopeRing {
                    shown: root.onNotes
                    anchors.fill: parent
                }

                HoverHandler {
                    enabled: root.albumNotes.length > 0
                    cursorShape: Qt.PointingHandCursor
                }
                TapHandler {
                    enabled: root.albumNotes.length > 0
                    onTapped: {
                        root.clearHeaderStops(false);
                        root.onNotes = true;
                        root.setNotesExpanded(!root.notesExpanded);
                    }
                }
            }
        }

        // The bottom section is the scroll view alone: the track count,
        // runtime and format now live as fineprint under the album title.
        ColumnLayout {
            id: tracksSection
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: root.height * 0.008

            ScrollContent {
                id: content
                Layout.fillWidth: true
                Layout.fillHeight: true
                model: songs
                selectable: true
                memoryKey: root.memoryKey
                quickActionKind: "song"
                onQuickAction: (target, actions) => root.quickAction(target, actions)
                visible: !(root.loading && songs.count === 0)
                // ARC-UI-2 M1: the rows paint track, title and duration.
                searchFields: ["track", "title", "subtitle", "duration"]
                // UI-58: rows scale from the page height, not the song viewport
                // (the header/notes reflow must not rescale every row). The
                // precomputed extent keeps the shared glide off Qt's lazy
                // estimate.
                context: "album"
                // Short albums sit against the page bottom instead of floating at
                // the top of the scroller.
                bottomAlignShort: true
                rowRefHeight: root.height
                onRowTapped: (index, shift) => {
                    if (shift) {
                        root.clearHeaderStops();
                        content.extendSelectionTo(index);
                        return;
                    }
                    root.clearHeaderStops();
                    content.clearSelection();
                    content.currentIndex = index;
                    root.playRow(index);
                }

                rowDelegate: Component {
                    Rectangle {
                        id: songRow
                        radius: Theme.radius
                        required property int index
                        required property string track
                        required property string title
                        required property string subtitle
                        required property string duration
                        required property string songId
                        required property string albumId
                        required property string cover
                        required property bool starred
                        required property bool playable

                        // VI-4: a row inside the visual selection wears the cursor fill.
                        readonly property bool selectedRow: content.selected(index)
                        // Attached `ListView.isCurrentItem` only resolves on
                        // the delegate root; children read this instead.
                        readonly property bool currentItem: ListView.isCurrentItem && !root.onToolbar
                        readonly property bool emphasized: ListView.isCurrentItem || songRow.index === content.hoveredIndex

                        width: ListView.view ? ListView.view.width : 0
                        z: emphasized ? 1 : 0
                        // UI-58: the model's page-relative adaptive row.
                        height: content.adaptiveRowHeight
                        color: "transparent"
                        opacity: songRow.playable ? 1 : 0.45

                        // The shared fill layer: zebra, hover, cursor and range.
                        RowHighlight {
                            index: songRow.index
                            striped: true
                            baseColor: Palette.surface
                            current: songRow.currentItem
                            hovered: songRow.index === content.hoveredIndex
                            selected: songRow.selectedRow
                        }

                        // The shared list row: AlbumEntry is the no-cover context.
                        RowBody {
                            anchors.fill: parent
                            viewWidth: songRow.width
                            rowHeight: songRow.height
                            metrics: rowMetrics
                            emphasized: songRow.emphasized
                            current: songRow.currentItem
                            showCover: root.isPlaylist
                            cover: songRow.cover
                            showSubtitle: songRow.subtitle.length > 0
                            title: songRow.track.length > 0 ? songRow.track + ". " + songRow.title : songRow.title
                            subtitle: songRow.subtitle
                            meta: songRow.duration
                            reserveMeta: true
                            metaAlignRight: true
                            leftMargin: songRow.width * 0.02
                            textGap: songRow.width * 0.02
                            rightMargin: songRow.width * 0.02
                            marksMargin: songRow.width * 0.01275
                            markCursor: content.currentIndex
                            visibleStart: content.firstVisibleIndex
                            visibleEnd: content.lastVisibleIndex
                            markIndex: songRow.index
                            markSongId: songRow.songId
                            markAlbumId: songRow.albumId
                            markStarred: songRow.starred
                            markRatingVisible: false
                            searchQuery: content.searchQuery
                            searchCurrent: songRow.index === content.searchCurrentRow
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
}
