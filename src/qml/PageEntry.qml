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
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)
    signal descriptionRequested()

    /// "album" or "playlist": the same page anatomy, different source and art.
    property string entryKind: "album"
    property string albumId: ""
    property string playlistId: ""
    property string playlistTitle: ""
    /// ARC-RS-1: the radio page uses this shell with a similar-song source.
    property string radioSeedKind: ""
    property string radioSeedId: ""
    property string artistId: ""
    property string artistName: ""
    property string artistCover: ""
    property string artistMeta: ""
    property int artistAlbumCount: 0
    property int artistDuration: 0
    property string artistYear: ""
    property string artistDescription: ""
    property int artistRating: 0
    property bool artistStarred: false
    property bool artistCacheOn: false
    property bool artistDownloading: false
    property int chipScope: -1
    readonly property bool isArtist: root.entryKind === "artist"
    property string artistRowMode: "albums"
    readonly property bool isPlaylist: root.entryKind === "playlist"
    readonly property bool isRadio: root.entryKind === "radio"
    readonly property bool isAlbum: root.entryKind === "album"
    readonly property string pageId: root.isArtist ? root.artistId
        : root.isRadio ? root.radioSeedId
        : root.isPlaylist ? root.playlistId : root.albumId
    readonly property string rowMode: root.isArtist ? root.artistRowMode : "tracks"
    readonly property bool artistAlbums: root.isArtist && root.artistRowMode === "albums"
    readonly property bool exitsToMain: root.isArtist
    readonly property string pageTitle: albumTitle
    readonly property string pageSubtitle: playlistSubtitle
    readonly property string pageArtist: albumArtist
    readonly property string pageMeta: albumMeta
    readonly property string pageCover: albumCover
    readonly property bool pageStarred: albumStarred
    readonly property int pageRating: albumRating
    readonly property bool pageCacheOn: albumCacheOn
    readonly property bool pageDownloading: artistDownloading
    readonly property string pageDescription: albumNotes
    readonly property string metaLine: root.albumMetaLine
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
    readonly property bool hasDescription: root.pageDescription.length > 0
    readonly property string descriptionTitle: root.isArtist ? root.artistName : root.albumTitle
    readonly property string descriptionSubtitle: root.isArtist ? root.artistMeta : root.albumMeta
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
    // UI-22: true while the album's songs are in flight.
    property bool loading: false
    // UI-54: per-album/per-playlist scroll memory for the songs viewport.
    readonly property string memoryKey: root.isRadio
        ? "radio:" + root.radioSeedId
        : root.isArtist ? "artist:" + root.artistId
        : root.isPlaylist ? "playlist:" + root.playlistId : "album:" + root.albumId
    property bool _restored: false
    /// A session-restored page starts on row 0, like a scope or order reset.
    property bool resetCursorOnOpen: false
    property bool _focusApplied: false
    property int _coverTarget: 0
    // Album whose rows request is in flight; avoids reload storms from
    // duplicate open paths and library-changed notifications.
    property string _loadingKey: ""
    property bool _opening: false
    property bool _ready: false

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
    /// Whole-hour or whole-minute runtime for page headers.
    function durationSpan(seconds) {
        var total = Number(seconds) || 0;
        if (total <= 0)
            return "";
        if (total < 3600)
            return Theme.countLabel(Math.max(1, Math.round(total / 60)), "Minute");
        return Theme.countLabel(Math.max(1, Math.round(total / 3600)), "Hour");
    }
    readonly property string tracksSpan: root.durationSpan(root.tracksDuration)
    readonly property string artistSpan: root.durationSpan(root.artistDuration)
    readonly property string artistHeaderStats: {
        var parts = [];
        if (root.artistAlbumCount > 0)
            parts.push(Theme.countLabel(root.artistAlbumCount, "album"));
        if (root.artistYear.length > 0)
            parts.push(root.artistYear);
        if (root.artistSpan.length > 0)
            parts.push(root.artistSpan);
        return Theme.metaJoin(parts);
    }
    readonly property string albumHeaderStats: {
        var parts = [];
        if (songs.count > 0)
            parts.push(Theme.countLabel(songs.count, "Track"));
        if (root.albumMeta.length > 0)
            parts.push(root.albumMeta);
        if (root.tracksSpan.length > 0)
            parts.push(root.tracksSpan);
        return Theme.metaJoin(parts);
    }
    /// Album header: count, year, runtime. Artist keeps the album count.
    readonly property string albumMetaLine: root.isAlbum
        ? root.albumHeaderStats
        : root.isArtist
            ? root.albumMeta
            : Theme.metaJoin([root.albumMeta, root.tracksSummary])

    /// Star hitbox side in the header (five slots total). 60% larger than
    /// the old strip, matching the album-title scale.
    // The page header grows with the available column width, not only the
    // window height: wide layouts get the larger hero typography.
    readonly property real headerScale: Math.max(0.9, Math.min(1.6, root.width / 1280))
    readonly property real starUnit: Math.max(22, Math.min(60, root.height * 0.04 * root.headerScale))

    /// The header gap follows the cover but never eats more than 75% of
    /// the usable page height; wide pages keep room for the rows.
    readonly property real headerCoverBottom: Math.min(
        (root.isAlbum || root.isArtist ? coverBackdrop.coverSide : playlistArt.coverSide) - root.width * 0.02,
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
    function toggleRowScope() {
        if (!root.isArtist)
            return false;
        root.artistRowMode = root.artistAlbums ? "tracks" : "albums";
        songs.clear();
        content.clearSelection();
        content.currentIndex = -1;
        root._loadingKey = "";
        root.reload();
        return true;
    }

    function toggleArtistCache() {
        root.artistCacheOn = !root.artistCacheOn;
        root.artistDownloading = root.artistCacheOn;
        Backend.downloadArtist(root.artistId, root.artistCacheOn);
    }

    function toggleArtistFavorite() {
        root.artistStarred = !root.artistStarred;
        root.albumStarred = root.artistStarred;
        Backend.starArtist(root.artistId, root.artistStarred);
    }

    function shuffleArtist() {
        if (root.artistId.length > 0)
            Backend.shuffleArtist(root.artistId);
    }

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
        if (root.isArtist) {
            var artist = {
                artistId: root.artistId,
                starred: root.artistStarred,
                rating: root.artistRating
            };
            var actions = verbs.forItem("artist", artist, false, function (level) {
                root.artistRating = level;
                root.albumRating = level;
            });
            verbs.wrapFavorite(actions, function (on) {
                root.artistStarred = on;
                root.albumStarred = on;
            });
            return actions;
        }
        return verbs.album(root.albumItem(), root.albumCacheOn, function (level) {
            root.albumRating = level;
        });
    }

    function armCoverActions(sticky) {
        if (root.isRadio)
            return;
        if (root.isArtist) {
            if (root.artistId.length === 0)
                return;
            var artistActions = root.coverActions();
            artistActions.sticky = sticky === true;
            root.quickAction(favoriteChip, artistActions);
            return;
        }
        if (root.isPlaylist) {
            if (root.playlistId.length === 0)
                return;
            var playlistActions = verbs.playlist({ playlistId: root.playlistId });
            playlistActions.sticky = sticky === true;
            root.quickAction(coverBox, playlistActions);
            return;
        }
        if (root.albumId.length === 0)
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
        if (root.isPlaylist || (root.albumId === "" && root.artistId === ""))
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
        if (root.isArtist)
            Backend.requestArtistCover(root.artistId, target);
        else
            Backend.requestAlbumCover(root.albumId, target);
    }

    function reload() {
        if (root.isRadio) {
            if (root.radioSeedId === "" || root._loadingKey === "radio:" + root.radioSeedId)
                return;
            root._loadingKey = "radio:" + root.radioSeedId;
            root.loading = true;
            root.onNotes = false;
            Backend.requestRadio(root.radioSeedKind, root.radioSeedId);
            return;
        }
        if (root.isArtist) {
            var artistKey = root.artistAlbums
                ? "artist:" + root.artistId : "artist-tracks:" + root.artistId;
            if (root.artistId === "" || root._loadingKey === artistKey)
                return;
            root._loadingKey = artistKey;
            root.loading = true;
            root.onNotes = false;
            if (root.artistAlbums) {
                Backend.requestArtist(root.artistId);
                Backend.requestArtistUser(root.artistId);
                Backend.requestArtistInfo(root.artistId);
                Backend.requestArtistCache(root.artistId);
                root._coverTarget = 0;
                root.scheduleCover();
            } else {
                Backend.requestArtistSongs(root.artistId);
            }
            return;
        }
        if (root.isPlaylist) {
            if (root.playlistId === "" || root._loadingKey === "playlist:" + root.playlistId)
                return;
            root._loadingKey = "playlist:" + root.playlistId;
            root.loading = true;
            root.onNotes = false;
            Backend.requestPlaylist(root.playlistId);
            return;
        }
        if (root.albumId === "")
            return;
        // One request per album at a time. Startup can deliver a restored
        // page and a library-changed notification in the same turn; the
        // second must not bump the first payload away.
        if (root._loadingKey === "album:" + root.albumId)
            return;
        root._loadingKey = "album:" + root.albumId;
        root.loading = true;
        root.onNotes = false;
        Backend.requestAlbumSongs(root.albumId);
        Backend.requestAlbumInfo(root.albumId);

        // The display copy is requested from the painted cover box (DPR +
        // screen-capped bucket); the original stays on screen until it lands.
        root._coverTarget = 0;
        root.scheduleCover();

        Backend.requestAlbumState(root.albumId);
        Backend.requestAlbumCache(root.albumId);
    }

    /// Reuses this page for another album/playlist. The page object, header
    /// and ScrollPanel stay alive; only the row source and header data swap.
    /// Returns false when the requested page is already current.
    function open(page) {
        var nextPlaylist = page.playlistId ? String(page.playlistId) : "";
        var nextAlbum = page.albumId ? String(page.albumId) : "";
        var nextArtist = page.artistId ? String(page.artistId) : "";
        var nextRadio = page.radioSeedId ? String(page.radioSeedId) : "";
        var nextKind = nextRadio.length > 0 ? "radio"
            : nextArtist.length > 0 ? "artist"
            : nextPlaylist.length > 0 ? "playlist" : "album";
        if (root.entryKind === nextKind
                && root.albumId === nextAlbum
                && root.playlistId === nextPlaylist
                && root.artistId === nextArtist
                && root.radioSeedId === nextRadio) {
            if (page.focusSongId) {
                root.focusSongId = String(page.focusSongId);
                root._focusApplied = false;
                root.focusRequestedSong();
            }
            return false;
        }

        root._opening = true;
        root._loadingKey = "";
        songs.clear();
        content.clearSelection();
        content.currentIndex = -1;
        root.entryKind = nextKind;
        root.albumId = nextAlbum;
        root.playlistId = nextPlaylist;
        root.radioSeedKind = page.radioSeedKind ? String(page.radioSeedKind) : ""
        root.radioSeedId = nextRadio;
        root.albumTitle = page.albumTitle ? String(page.albumTitle) : ""
        root.albumArtist = page.albumArtist ? String(page.albumArtist) : ""
        root.albumCover = page.albumCover ? String(page.albumCover) : ""
        root.albumMeta = page.albumMeta ? String(page.albumMeta) : ""
        root.albumStarred = page.albumStarred === true
        root.albumRating = 0;
        root.albumCacheOn = false;
        root.albumNotes = "";
        root.playlistSubtitle = page.playlistTitle ? String(page.playlistTitle) : ""
        root.playlistCollage = [];
        root.artistId = nextArtist
        root.artistName = page.artistName ? String(page.artistName) : ""
        root.artistCover = page.artistCover ? String(page.artistCover) : ""
        root.artistMeta = ""
        root.artistRowMode = "albums"
        root.artistAlbumCount = 0
        root.artistDuration = 0
        root.artistYear = ""
        root.artistDescription = ""
        root.artistRating = 0
        root.artistStarred = false
        root.artistCacheOn = false
        root.artistDownloading = false
        root.chipScope = -1
        if (nextKind === "artist") {
            root.albumTitle = root.artistName
            root.albumCover = root.artistCover
            root.albumMeta = root.artistMeta
            root.albumMeta = ""
        }
        root.tracksDuration = 0;
        root.tracksFormat = "";
        root.loading = true;
        root.focusSongId = page.focusSongId ? String(page.focusSongId) : ""
        root.resetCursorOnOpen = page.resetCursorOnOpen === true
        root._restored = false;
        root._focusApplied = false;
        root._coverTarget = 0;
        root.onCover = false;
        root.onArtist = false;
        root.onNotes = false;
        root.onToolbar = false;
        root._opening = false;
        root.reload()
        return true;
    }

    function applyInfo(payload) {
        var info = JSON.parse(payload);
        if (root.isArtist) {
            if (info.id !== root.artistId)
                return;
            root.artistDescription = info.biography ? String(info.biography) : "";
            root.albumNotes = root.artistDescription;
            return;
        }
        if (info.album_id && info.album_id !== root.albumId)
            return;
        root.albumNotes = info.notes ? info.notes : "";
        if (root.albumNotes.length === 0) {
            root.onNotes = false;
        }
    }

    /// Resolve the album's owning artist and let Home push its page.
    function activateArtist() {
        if (!root.isAlbum || root.albumArtist.length === 0)
            return;
        Backend.requestAlbumArtist(root.albumId);
    }

    /// Clear every header Tab stop so the rows regain focus; the notes
    /// reader collapses too unless a caller is about to toggle it.
    function clearHeaderStops() {
        root.onCover = false;
        root.onArtist = false;
        root.onNotes = false;
        root.onToolbar = false;
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

    function applyArtist(payload) {
        var artist = JSON.parse(payload);
        if (!root.isArtist || artist.id !== root.artistId)
            return;
        root._loadingKey = "";
        root.artistName = artist.name ? String(artist.name) : root.artistName;
        if (artist.cover && artist.cover.length > 0)
            root.artistCover = String(artist.cover);
        root.albumTitle = root.artistName;
        root.albumCover = root.artistCover;
        root.albumNotes = root.artistDescription;
        root.artistRating = root.albumRating;
        root.artistStarred = root.albumStarred;
        content.clearSelection();
        songs.clear();
        var rows = artist.albums ? artist.albums : [];
        var firstYear = 0;
        var lastYear = 0;
        var totalDuration = 0;
        for (var i = 0; i < rows.length; i++) {
            var album = rows[i];
            if (album.year) {
                var year = Number(album.year) || 0;
                if (year > 0 && (firstYear === 0 || year < firstYear))
                    firstYear = year;
                if (year > lastYear)
                    lastYear = year;
            }
            totalDuration += Number(album.duration) || 0;
            var meta = [];
            if (album.year)
                meta.push(String(album.year));
            meta.push(Theme.countLabel(album.song_count, "song"));
            songs.append({
                track: "",
                title: album.title ? String(album.title) : "",
                subtitle: album.artist ? String(album.artist) : "",
                duration: Theme.metaJoin(meta),
                songId: "",
                albumId: album.id ? String(album.id) : "",
                cover: album.cover ? String(album.cover) : "",
                starred: album.starred === true,
                playable: true
            });
        }
        root.artistAlbumCount = rows.length;
        root.artistDuration = totalDuration;
        root.artistYear = firstYear === 0 ? ""
            : (firstYear === lastYear ? String(firstYear) : firstYear + "–" + lastYear);
        root.artistMeta = root.artistHeaderStats;
        root.albumMeta = root.artistMeta;
        root.loading = false;
        if (songs.count > 0 && (content.currentIndex < 0 || content.currentIndex >= songs.count))
            content.currentIndex = 0;
        content.restore();
        root.scheduleCover();
    }

    function applyArtistTracks(payload) {
        var data = JSON.parse(payload);
        if (!root.isArtist || data.artist_id !== root.artistId)
            return;
        root._loadingKey = "";
        content.clearSelection();
        songs.clear();
        var rows = data.songs ? data.songs : [];
        for (var i = 0; i < rows.length; i++) {
            var row = rows[i];
            songs.append({
                track: row.track ? String(row.track) : "",
                title: row.title ? String(row.title) : "",
                subtitle: row.subtitle ? String(row.subtitle) : "",
                duration: row.meta ? String(row.meta) : "",
                songId: row.songId ? String(row.songId) : "",
                albumId: row.albumId ? String(row.albumId) : "",
                cover: row.cover ? String(row.cover) : "",
                starred: row.starred === true,
                playable: true
            });
        }
        root.loading = false;
        if (songs.count > 0 && (content.currentIndex < 0 || content.currentIndex >= songs.count))
            content.currentIndex = 0;
        content.restore();
    }

    function applyPlaylist(payload) {
        var data = JSON.parse(payload);
        if (data.playlist_id !== root.playlistId)
            return;
        root._loadingKey = "";
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

    /// ARC-RS-1: the similar-song payload. Radio rows are server tracks, so
    /// the header keeps the seed identity and the cover art is a collage.
    function applyRadio(payload) {
        var data = JSON.parse(payload);
        if (data.seed_id !== root.radioSeedId || data.seed_kind !== root.radioSeedKind)
            return;
        root._loadingKey = "";
        root.albumTitle = data.title ? String(data.title) : "";
        root.playlistSubtitle = data.subtitle ? String(data.subtitle) : "";
        root.albumMeta = data.meta ? String(data.meta) : "";
        root.playlistCollage = data.collage ? data.collage : [];
        root.tracksDuration = Number(data.duration) || 0;
        root.tracksFormat = "";
        content.clearSelection();
        songs.clear();
        var rows = data.songs ? data.songs : [];
        for (var i = 0; i < rows.length; i++) {
            var row = rows[i];
            songs.append({
                track: "",
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
        root._loadingKey = "";
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
        if (!root.isAlbum)
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

    onAlbumIdChanged: if (root._ready && !root._opening && root.isAlbum) Qt.callLater(root.reload)
    onArtistIdChanged: if (root._ready && !root._opening && root.isArtist) Qt.callLater(root.reload)
    onPlaylistIdChanged: if (root._ready && !root._opening && root.isPlaylist) Qt.callLater(root.reload)
    onRadioSeedIdChanged: if (root._ready && !root._opening && root.isRadio) Qt.callLater(root.reload)
    onEntryKindChanged: if (root._ready && !root._opening && !root.isAlbum) Qt.callLater(root.reload)
    Component.onCompleted: {
        root._ready = true;
        reload();
    }

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
        if (root.isArtist) {
            if (root.artistAlbums) {
                var albumRow = songs.get(index);
                if (albumRow && albumRow.albumId)
                    root.albumActivated(albumRow.albumId, albumRow.title, albumRow.subtitle, "", "", albumRow.cover, albumRow.starred === true);
            } else {
                Backend.playArtist(root.artistId, index);
            }
            return;
        }
        if (root.isRadio) {
            Backend.playRadio(root.radioSeedKind, root.radioSeedId, index);
            return;
        }
        if (root.isPlaylist) {
            var row = songs.get(index);
            if (row && row.playable === true)
                Backend.playPlaylist(root.playlistId, index);
            return;
        }
        Backend.playAlbum(root.albumId, index);
    }

    function handleArtistAction(action) {
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            var state = root.onCover ? 1 : (root.onNotes ? 2 : (root.chipScope >= 0 ? 3 + root.chipScope : 0));
            do {
                state = (state + (action === "pane_cycle" ? 1 : 5)) % 6;
            } while (state === 2 && root.albumNotes.length === 0)
            root.onCover = state === 1;
            root.onNotes = state === 2;
            root.chipScope = state >= 3 ? state - 3 : -1;
            if (state !== 0)
                content.clearSelection();
            else if (songs.count > 0 && content.currentIndex < 0)
                content.currentIndex = 0;
            return true;
        }
        if (root.chipScope >= 0) {
            if (action === "enter")
                root.activateArtistChip();
            else if (action === "move_up") {
                root.chipScope = root.chipScope > 0 ? root.chipScope - 1 : 2;
                if (root.chipScope === 2 && root.albumNotes.length > 0)
                    root.onNotes = true;
            } else if (action === "move_down") {
                root.chipScope = -1;
                root.onCover = true;
            }
            return true;
        }
        if (root.onCover || root.onNotes) {
            if (action === "enter")
                root.onCover ? root.armCoverActions(true) : root.descriptionRequested();
            else if (action === "move_down") {
                root.onCover = false;
                root.onNotes = false;
                root.chipScope = 0;
            } else if (action === "move_up") {
                root.onCover = false;
                root.onNotes = false;
                root.chipScope = 2;
            }
            return true;
        }
        return false;
    }

    function activateArtistChip() {
        if (root.chipScope === 0)
            root.toggleArtistCache();
        else if (root.chipScope === 1)
            root.shuffleArtist();
        else if (root.chipScope === 2)
            root.toggleArtistFavorite();
    }

    function handleAction(action) {
        if (root.isArtist && root.handleArtistAction(action))
            return;
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
            if (action === "move_up") {
                root.onNotes = false;
                if (root.pageArtist.length > 0)
                    root.onArtist = true;
                else
                    root.onCover = true;
            } else if (action === "move_down") {
                root.onNotes = false;
                root.onToolbar = true;
            } else if (action === "enter") {
                root.descriptionRequested();
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
            root.playRow(content.currentIndex);
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artist_songs") {
                root.applyArtistTracks(payload);
            } else if (channel === "artist") {
                root.applyArtist(payload);
            } else if (channel === "artist_info") {
                root.applyInfo(payload);
            } else if (channel === "artist_user") {
                var user = JSON.parse(payload);
                if (user.id === root.artistId) {
                    root.artistStarred = user.starred === true;
                    root.albumStarred = root.artistStarred;
                    root.artistRating = Number(user.rating) || 0;
                    root.albumRating = root.artistRating;
                }
            } else if (channel === "artist_cache") {
                var artistCache = JSON.parse(payload);
                if (artistCache.id === root.artistId) {
                    root.artistCacheOn = artistCache.on === true;
                    root.artistDownloading = artistCache.active === true;
                }
            } else if (channel === "artist_cover") {
                var artistArt = JSON.parse(payload);
                if (artistArt.artist_id === root.artistId && artistArt.cover
                        && Number(artistArt.size) === root._coverTarget)
                    root.albumCover = artistArt.cover;
            } else if (channel === "radio") {
                root.applyRadio(payload);
            } else if (channel === "playlist") {
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
        function onAlbum_order_changed() {
            if (root.isArtist)
                root.reload();
        }
        function onRatingChanged(kind, id, rating) {
            if (kind === "artist" && id === root.artistId) {
                root.artistRating = rating;
                root.albumRating = rating;
            }
        }
    }

    // Ambience (D-5): the album art itself, painted sharp with no wash
    CoverBackdrop {
        id: coverBackdrop
        anchors.fill: parent
        visible: root.isAlbum || root.isArtist
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
        visible: !(root.isAlbum || root.isArtist)
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
                    text: root.pageTitle
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.04 * root.headerScale
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
                    visible: !root.isAlbum
                    text: root.playlistSubtitle
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.024 * root.headerScale
                    elide: Text.ElideRight
                }

                StarRating {
                    visible: root.isAlbum || root.isArtist
                    Layout.alignment: Qt.AlignLeft
                    unit: root.starUnit
                    rating: root.albumRating
                    onPicked: level => {
                        root.albumRating = level;
                        if (root.isArtist)
                            Backend.rateArtist(root.artistId, level);
                        else
                            Backend.rateAlbum(root.albumId, level);
                    }
                }

                FadeText {
                    id: artistLink
                    visible: root.isAlbum
                    Layout.fillWidth: true
                    text: root.pageArtist
                    color: Palette.primary
                    font.pixelSize: root.height * 0.024 * root.headerScale
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

                    // ARC-I-2/UI-60: the album.s favourite leads the header
                    // meta (the star row above owns the rating now).
                    Item {
                        visible: (root.isAlbum || root.isArtist) && root.pageStarred
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
                        text: root.metaLine
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02 * root.headerScale
                        elide: Text.ElideRight
                    }

                    // VI-4: the selection count pill follows the metadata.
                    SelectionBadge {
                        Layout.alignment: Qt.AlignVCenter
                        count: root.selectionCount
                        pixelSize: root.height * 0.02
                    }
                }

                Row {
                    visible: root.isArtist
                    Layout.alignment: Qt.AlignLeft
                    spacing: root.height * 0.010

                    ActionChip {
                        unit: root.height * 0.06 * root.headerScale
                        icon: ""
                        onClicked: {
                            root.chipScope = 0;
                            root.clearHeaderStops(false);
                            root.toggleArtistCache();
                        }

                        DownloadButton {
                            anchors.centerIn: parent
                            base: parent.height * 0.42
                            fetching: root.artistDownloading
                            done: root.artistCacheOn
                            showIdle: true
                        }

                        ScopeRing { shown: root.isArtist && root.chipScope === 0 }
                    }

                    ActionChip {
                        unit: root.height * 0.06 * root.headerScale
                        icon: "qrc:/app/icons/outline-shuffle.svg"
                        onClicked: {
                            root.chipScope = 1;
                            root.clearHeaderStops(false);
                            root.shuffleArtist();
                        }
                        ScopeRing { shown: root.isArtist && root.chipScope === 1 }
                    }

                    ActionChip {
                        id: favoriteChip
                        unit: root.height * 0.06 * root.headerScale
                        icon: ""
                        onClicked: {
                            root.chipScope = 2;
                            root.clearHeaderStops(false);
                            root.toggleArtistFavorite();
                        }
                        FavoriteButton {
                            anchors.fill: parent
                            variant: "chip"
                            unit: parent.height * 0.42
                            starred: root.artistStarred
                            rating: root.artistRating
                            showCount: false
                        }
                        ScopeRing { shown: root.isArtist && root.chipScope === 2 }
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
                visible: (root.isAlbum || root.isArtist) && root.albumNotes.length > 0
                clip: true

                FadeText {
                    id: notesText
                    anchors.fill: parent
                    text: root.pageDescription
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                    lineHeight: 1.25
                    wrapMode: Text.WordWrap
                    maximumLineCount: 3
                    elide: Text.ElideRight
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
                        root.descriptionRequested();
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
                quickActionKind: root.artistAlbums ? "album" : "song"
                markSongs: !root.artistAlbums
                markAlbums: root.artistAlbums
                onQuickAction: (target, actions) => root.quickAction(target, actions)
                visible: !(root.loading && songs.count === 0)
                // ARC-UI-2 M1: the rows paint track, title and duration.
                searchFields: root.artistAlbums ? ["title", "subtitle", "meta"] : ["track", "title", "subtitle", "duration"]
                // UI-58: rows scale from the page height, not the song viewport
                // (the header/notes reflow must not rescale every row). The
                // precomputed extent keeps the shared glide off Qt's lazy
                // estimate.
                context: root.artistAlbums ? "library" : "album"
                // Short albums sit against the page bottom instead of floating at
                // the top of the scroller.
                bottomAlignShort: !root.artistAlbums
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

                        // The shared list row: PageEntry is the no-cover context.
                        RowBody {
                            anchors.fill: parent
                            viewWidth: songRow.width
                            rowHeight: songRow.height
                            metrics: rowMetrics
                            emphasized: songRow.emphasized
                            current: songRow.currentItem
                            showCover: root.isArtist ? root.artistAlbums : !root.isAlbum
                            cover: songRow.cover
                            showSubtitle: songRow.subtitle.length > 0
                            title: root.artistAlbums ? songRow.title
                                : (songRow.track.length > 0 ? songRow.track + ". " + songRow.title : songRow.title)
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
                            markRatingVisible: root.artistAlbums
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
                text: root.isRadio
                    ? (Backend.offline ? "Radio needs a connection" : "No similar songs found")
                    : "No tracks in this album"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("body", root.height)
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }
    }
}
