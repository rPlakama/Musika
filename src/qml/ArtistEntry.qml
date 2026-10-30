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
import musika
import "Theme.js" as Theme
import "CoverSize.js" as CoverSize

// Artist entry (P-8, milestone A): round avatar + name over the album
// list. Pushed from ArtistList and from artist hits in search; Enter on a
// row opens the album page, Enter/Back
// from here always returns to Main (see Home.leaveArtistEntry). Favorite,
// stars, biography and the cache rule land in milestones B-D.
Rectangle {
    radius: Theme.radius
    id: root

    RowMetrics {
        id: rowMetrics
        viewWidth: wall.width
        rowHeight: Math.max(48, root.height * 0.09)
    }

    signal back
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)

    // ARC-43/ARC-UI-1: Alt-hold quick actions for an album card — four fixed
    // verbs, minus Go to Album (Enter already opens it): that slot stays the
    // open "—".
    signal quickAction(var target, var actions)

    /// The artist album list rides the same ScrollPanel as the Main lists,
    /// with gg/G, the `/` contract, selection, memory and adaptive states.
    /// The list lives in `wall` below.
    readonly property bool hasSelection: wall.hasSelection
    readonly property int selectionCount: wall.selectionCount

    /// VI-4: `v` toggles visual mode on the cursor card.
    function toggleSelection() {
        wall.toggleSelection();
    }

    /// VI-4: `V` selects every card.
    function selectAll() {
        wall.selectAll();
    }

    function clearSelection() {
        wall.clearSelection();
    }

    /// ARC-UI-2 M2: the live `/` session, painted by the card/row text.
    property alias searchQuery: wall.searchQuery
    property alias searchCurrentRow: wall.searchCurrentRow
    /// The view cursor, exposed so Home's up-edge can hand it to the status
    /// region at the first card (the other surfaces alias `currentIndex`).
    readonly property alias currentIndex: wall.currentIndex

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`); the wall
    /// paints the title and the meta line. Matching is viewport-scoped: cards
    /// scrolled out of the viewport come back empty and never match.
    function searchRows() {
        return wall.searchRows();
    }

    function searchJump(row) {
        wall.searchJump(row);
    }

    /// ARC-I-4 (decoration only): the Favorite verb stars on the server and
    /// emits nothing back, so light the card's heart the moment the verb
    /// fires — no DB check, no reload; the next payload carries the truth and
    /// can clear it again.
    // ARC-I-4: the favourite write lives in the shared scroll model now
    // (`wall.markStarred`) — cursor card or the whole selected range.

    /// ARC-43/ARC-UI-1: Alt armed the gesture — the scoped favourite chip
    /// rates the artist itself; otherwise the shared wall anchors on the
    /// cursor card with the recycled album verbs (or the selection's batch).
    function armQuickActions() {
        if (root.chipScope === 2) {
            var artist = {
                artistId: root.artistId,
                starred: root.artistStarred,
                rating: root.artistRating
            };
            var artistActions = verbs.forItem("artist", artist, false, function (level) {
                root.artistRating = level;
            });
            verbs.wrapFavorite(artistActions, function (on) {
                root.artistStarred = on;
            });
            quickAction(favoriteChip, artistActions);
            return;
        }
        wall.armQuickActions();
    }

    property string artistId: ""
    property string artistName: ""
    property string artistCover: ""
    property string artistMeta: ""
    // UI-52: biography slot for the header's identity strip; ARC-42 fills it
    // from the `artist_info` channel. Empty shows the placeholder copy.
    property string artistDescription: ""
    /// What the strip actually draws (placeholder when no biography yet).
    readonly property string descriptionText: artistDescription.length > 0
        ? artistDescription
        : "Artist biography will appear here once the server metadata is wired. Or Such artist doesn't contain a biography at all;"
    // UI-52: current artist rating, 0..5 (Subsonic `setRating` domain);
    // 0 = unrated. ARC-42 fetches and toggles it.
    property int artistRating: 0
    // UI-52/ARC-42: favourite state from `artist_user.db`.
    property bool artistStarred: false
    // CACHE-5/8: the artist container is selected for download (rule in
    // media.db + marks in download.db); the In-cache chip toggles it.
    property bool artistCacheOn: false
    // CACHE-5: the fetch pass for this artist is running (the chip shows the
    // spinning loading loop while the ledger works through the songs).
    property bool artistDownloading: false
    // UI-54: per-artist scroll memory for the album list.
    readonly property string memoryKey: "artist:" + root.artistId
    property int _coverTarget: 0
    // A display copy already won the race against the full `artist` payload.
    property bool _coverFromDisplay: false

    // Home uses this to send `Esc` straight to Main (P-8) instead of popping
    // back to the Artists segment or a search result.
    readonly property bool exitsToMain: true

    // Study §2 "name+server tag": host only, never the full URL.
    readonly property string serverTag: {
        var url = Backend.serverURL;
        var mark = url.indexOf("://");
        var rest = mark >= 0 ? url.substring(mark + 3) : url;
        var slash = rest.indexOf("/");
        return slash >= 0 ? rest.substring(0, slash) : rest;
    }

    color: Palette.surface


    RowModel {
        id: virtualAlbums
    }

    QuickActions {
        id: verbs
    }

    function scheduleCover() {
        if (root.artistId === "")
            return;
        coverTimer.restart();
    }

    function requestCoverNow() {
        var target = CoverSize.target(coverBackdrop.coverSide, Screen.devicePixelRatio,
            Math.max(Screen.width, Screen.height));
        if (target <= 0) {
            root._coverTarget = 0;
            return;
        }
        if (target === root._coverTarget)
            return;
        root._coverTarget = target;
        Backend.requestArtistCover(root.artistId, target);
    }

    function reload() {
        if (root.artistId === "")
            return;
        Backend.requestArtist(root.artistId);
        // ARC-42: user state (local index) + biography (library.db cache;
        // the request refreshes from the server when reachable).
        Backend.requestArtistUser(root.artistId);
        Backend.requestArtistInfo(root.artistId);
        Backend.requestArtistCache(root.artistId);
        // The page art uses a screen/DPR bucket; the original stays on
        // screen until the display copy lands.
        root._coverTarget = 0;
        root._coverFromDisplay = false;
        root.scheduleCover();
    }

    // CACHE-5/8: the stored container selection (`artist_cache` channel).
    function applyCache(payload) {
        var cache = JSON.parse(payload);
        if (cache.id !== root.artistId)
            return;
        root.artistCacheOn = cache.on === true;
        root.artistDownloading = cache.active === true;
    }

    // ARC-42: local favourite/rating state for this artist.
    function applyUser(payload) {
        var user = JSON.parse(payload);
        if (user.id !== root.artistId)
            return; // stale response for an artist we already left
        root.artistStarred = user.starred === true;
        root.artistRating = Number(user.rating) || 0;
    }

    // ARC-42: cached/refreshed biography for this artist.
    function applyInfo(payload) {
        var info = JSON.parse(payload);
        if (info.id !== root.artistId)
            return;
        root.artistDescription = info.biography ? info.biography : "";
    }

    function apply(payload) {
        // VI-4: a reload invalidates the selected range.
        wall.clearSelection();
        var artist = JSON.parse(payload);
        if (artist.name && artist.name.length > 0)
            root.artistName = artist.name;
        if (artist.cover && artist.cover.length > 0 && !root._coverFromDisplay)
            root.artistCover = artist.cover;
        root.artistMeta = Theme.countLabel(artist.album_count, "album");

        virtualAlbums.setSurface("artist_albums");
        virtualAlbums.setFilter(root.artistId);
        virtualAlbums.setOrder("Year");
        virtualAlbums.ensure(0, Theme.rowBudget - 1);
        if (wall.count > 0 && (wall.currentIndex < 0 || wall.currentIndex >= wall.count))
            wall.currentIndex = 0;
        // UI-54: reopening the entry restores the cursor + offset (and the
        // ARC-UI-3 layout); later reloads keep the live state.
        wall.restore();
        root.scheduleCover();
    }

    function ensureAlbums() {
        if (root.artistId === "" || wall.firstVisibleIndex < 0
                || wall.lastVisibleIndex < wall.firstVisibleIndex)
            return;
        virtualAlbums.ensure(Math.max(0, wall.firstVisibleIndex - Theme.rowBudget),
            wall.lastVisibleIndex + Theme.rowBudget);
    }

    function activateCurrent() {
        var album = JSON.parse(virtualAlbums.row(wall.currentIndex));
        if (album)
            root.albumActivated(album.albumId, album.title, album.artist, album.year, album.tracks, album.cover, album.starred === true);
    }

    /// Header Tab stops: title, biography, then the chips (0 download,
    /// 1 shuffle, 2 favorite). -1 returns Enter to the album list; any
    /// list move drops the ring back there.
    property int chipScope: -1
    /// Tab stops above the chips: the artist title and the biography.
    property bool onTitle: false
    property bool onDescription: false

    function toggleCache() {
        root.artistCacheOn = !root.artistCacheOn;
        root.artistDownloading = root.artistCacheOn;
        Backend.downloadArtist(root.artistId, root.artistCacheOn);
    }

    function toggleFavorite() {
        root.artistStarred = !root.artistStarred;
        Backend.starArtist(root.artistId, root.artistStarred);
    }

    /// Enter on the scoped chip.
    function activateChip() {
        if (root.chipScope === 0)
            root.toggleCache();
        else if (root.chipScope === 1)
            root.shuffle();
        else if (root.chipScope === 2)
            root.toggleFavorite();
    }

    /// Tab order: title -> biography -> download -> shuffle -> favourite.
    function cycleHeader(step) {
        var i = root.onTitle ? 0 : (root.onDescription ? 1 : (root.chipScope >= 0 ? root.chipScope + 2 : -1));
        if (i < 0)
            i = 0;
        i = (i + step + 5) % 5;
        root.onTitle = i === 0;
        root.onDescription = i === 1;
        root.chipScope = i >= 2 ? i - 2 : -1;
    }

    function scrollDescription(delta) {
        if (!descriptionView)
            return;
        var max = Math.max(0, descriptionView.contentHeight - descriptionView.height);
        descriptionView.contentY = Math.max(0, Math.min(max, descriptionView.contentY + delta));
    }

    /// The shuffle chip: a time-seeded album rotation of this artist,
    /// queued and played from its head (the backend's seeded walk).
    function shuffle() {
        if (root.artistId.length > 0)
            Backend.shuffleArtist(root.artistId);
    }

    Timer {
        id: coverTimer
        interval: 120
        repeat: false
        onTriggered: root.requestCoverNow()
    }

    Connections {
        target: Screen
        function onDevicePixelRatioChanged() {
            root._coverTarget = 0;
            root.scheduleCover();
        }
    }

    Connections {
        target: wall
        function onFirstVisibleIndexChanged() { root.ensureAlbums() }
        function onLastVisibleIndexChanged() { root.ensureAlbums() }
        function onCurrentIndexChanged() { root.ensureAlbums() }
    }

    Component.onCompleted: reload()

    // The shared panel owns movement (list steps ±1, gg/G snap through
    // `edgeJump`); this only tracks the header chip scope.

    // Chip scope and Enter-on-chip are the host's; everything else goes to
    // the shared panel (movement, gg/G, Enter on a row).
    // `Esc` is handled by Home (leaves the entry for Main regardless of
    // where it was opened from).
    function handleAction(action) {
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            root.cycleHeader(action === "pane_cycle" ? 1 : 4);
            return;
        }
        if (root.onDescription) {
            if (action === "move_up" || action === "move_down") {
                var line = Math.max(12, descriptionText.font.pixelSize * descriptionText.lineHeight);
                root.scrollDescription(action === "move_up" ? -line : line);
                return;
            }
            if (action === "page_up" || action === "page_down") {
                root.scrollDescription(action === "page_up"
                    ? -descriptionView.height * 0.9 : descriptionView.height * 0.9);
                return;
            }
            if (action === "enter")
                return;
            root.onDescription = false;
        }
        if (root.onTitle) {
            if (action === "enter")
                return;
            root.onTitle = false;
        }
        if (action === "enter" && root.chipScope >= 0) {
            root.activateChip();
            return;
        }
        wall.handleAction(action);
    }

    /// VI-8a: the artist page's shared wall owns the album rows.
    function yankRows(rowOnly) {
        wall.yankRows(rowOnly);
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artist")
                root.apply(payload);
            else if (channel === "artist_user")
                root.applyUser(payload);
            else if (channel === "artist_info")
                root.applyInfo(payload);
            else if (channel === "artist_cache")
                root.applyCache(payload);
            else if (channel === "artist_cover") {
                var art = JSON.parse(payload);
                // Reject a reply for an older width bucket.
                if (art.artist_id === root.artistId && art.cover
                        && Number(art.size) === root._coverTarget) {
                    root.artistCover = art.cover;
                    root._coverFromDisplay = true;
                }
            }
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onAlbum_order_changed() {
            root.reload();
        }
        // UI-60: the header rating reconciles here; the wall's MarkWindow
        // patches the matching album cards.
        function onRatingChanged(kind, id, rating) {
            if (kind === "artist" && id === root.artistId)
                root.artistRating = rating;
        }
    }

    // Ambience (D-5): the artist art painted sharp with no wash
    CoverBackdrop {
        id: coverBackdrop
        anchors.fill: parent
        source: root.artistCover
        onCoverSideChanged: root.scheduleCover()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.02
        spacing: root.height * 0.02

        // Header, AlbumEntry style: the artist art is the page ambience and
        // the identity gets the full width, anchored at the top.
        ColumnLayout {
            id: header
            Layout.fillWidth: true
            spacing: root.height * 0.008

            ColumnLayout {
                Layout.fillWidth: true
                spacing: root.height * 0.006

                FadeText {
                    id: artistTitle
                    Layout.fillWidth: true
                    text: root.artistName
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.04
                    font.bold: true
                    elide: Text.ElideRight

                    ScopeRing {
                        anchors.fill: parent
                        shown: root.onTitle
                    }
                }

                StarRating {
                    Layout.alignment: Qt.AlignLeft
                    unit: Math.max(22, Math.min(45, root.height * 0.04))
                    rating: root.artistRating
                    onPicked: level => {
                        root.artistRating = level;
                        Backend.rateArtist(root.artistId, level);
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.height * 0.008

                    // ARC-I-2/UI-60: the favourite leads the fineprint line.
                    Item {
                        visible: root.artistStarred
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
                        text: Theme.metaJoin([root.artistMeta, root.serverTag])
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02
                        elide: Text.ElideRight
                    }
                }

                // UI-60: the artist actions keep their Tab ring, now under the
                // identity instead of inside the old 30/70 header box.
                Row {
                    Layout.alignment: Qt.AlignLeft
                    spacing: root.height * 0.010

                    ActionChip {
                        unit: root.height * 0.06
                        icon: ""
                        onClicked: {
                            root.chipScope = 0;
                            root.toggleCache();
                        }

                        DownloadButton {
                            anchors.centerIn: parent
                            base: parent.height * 0.42
                            fetching: root.artistDownloading
                            done: root.artistCacheOn
                            showIdle: true
                        }

                        ScopeRing {
                            shown: root.chipScope === 0
                        }
                    }

                    ActionChip {
                        unit: root.height * 0.06
                        icon: "qrc:/app/icons/outline-shuffle.svg"
                        onClicked: root.chipScope = 1

                        ScopeRing {
                            shown: root.chipScope === 1
                        }
                    }

                    ActionChip {
                        id: favoriteChip
                        unit: root.height * 0.06
                        icon: ""
                        onClicked: {
                            root.chipScope = 2;
                            root.toggleFavorite();
                        }

                        FavoriteButton {
                            anchors.fill: parent
                            variant: "chip"
                            unit: parent.height * 0.42
                            starred: root.artistStarred
                            rating: root.artistRating
                            showCount: false
                        }

                        ScopeRing {
                            shown: root.chipScope === 2
                        }
                    }
                }

                // The biography is Tab-selectable and scrolls in place; the wall
                // below keeps its space even for a long text.
                Item {
                    id: descriptionBox
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.height * 0.22
                    clip: true

                    Flickable {
                        id: descriptionView
                        anchors.fill: parent
                        contentWidth: width
                        contentHeight: descriptionText.implicitHeight
                        boundsBehavior: Flickable.StopAtBounds
                        interactive: contentHeight > height
                        clip: true

                        FadeText {
                            id: descriptionText
                            width: descriptionView.width
                            text: root.descriptionText
                            color: Palette.onSurfaceVariant
                            font.pixelSize: root.height * 0.02
                            lineHeight: 1.25
                            wrapMode: Text.WordWrap
                            maximumLineCount: 0
                            elide: Text.ElideNone
                        }
                    }

                    ScopeRing {
                        anchors.fill: parent
                        shown: root.onDescription
                    }

                    HoverHandler {
                        cursorShape: Qt.PointingHandCursor
                    }
                    TapHandler {
                        onTapped: {
                            root.onTitle = false;
                            root.chipScope = -1;
                            root.onDescription = true;
                        }
                    }
                    WheelHandler {
                        onWheel: (wheel) => {
                            root.scrollDescription(-wheel.angleDelta.y);
                            wheel.accepted = true;
                        }
                    }
                }
            }
        }
        // Albums: the shared ScrollPanel the Main list segments use: rows
        // with title and year/count, gg/G, the `/` contract, selection,
        // memory and the empty/loading states.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            // VI-4: the selection count pill over the wall corner.
            SelectionBadge {
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: parent.width * 0.01
                count: root.selectionCount
                pixelSize: root.height * 0.02
                z: 2
            }

            ScrollPanel {
                id: wall
                anchors.fill: parent
                title: ""
                context: "library"
                headerRatio: 0
                model: virtualAlbums
                yankKind: "album"
                selectable: true
                memoryKey: root.memoryKey
                quickActionKind: "album"
                markSongs: false
                markAlbums: true
                searchFields: ["title", "meta"]
                emptyText: "No albums indexed yet"

                onItemActivated: (index) => root.activateCurrent()
                onRowTapped: (index, shift) => {
                    if (shift) {
                        wall.extendSelectionTo(index);
                        return;
                    }
                    wall.currentIndex = index;
                    root.activateCurrent();
                }
                onQuickAction: (target, actions) => root.quickAction(target, actions)
                rowDelegate: Component {
                    Rectangle {
                        radius: Theme.radius
                        id: row
                        required property int index
                        required property string albumId
                        required property string title
                        required property string artist
                        required property string year
                        required property string tracks
                        required property string meta
                        required property string cover
                        required property bool starred
                        required property int rating

                        readonly property bool current: ListView.isCurrentItem
                        readonly property bool selectedRow: wall.isSelected(row.index)

                        width: ListView.view.width
                        height: Math.max(48, root.height * 0.09)
                        color: "transparent"

                        RowHighlight {
                            current: row.current
                            hovered: row.index === wall.hoveredIndex
                            selected: row.selectedRow
                        }

                        // The shared list row (same anatomy as the library
                        // segments and the queue): cover, title/meta, marks.
                        RowBody {
                            anchors.fill: parent
                            viewWidth: row.width
                            rowHeight: row.height
                            metrics: rowMetrics
                            emphasized: row.current || row.index === wall.hoveredIndex
                            current: row.current
                            showCover: true
                            showSubtitle: false
                            cover: row.cover
                            title: row.title
                            meta: row.meta
                            reserveMeta: false
                            metaAlignRight: true
                            leftMargin: row.width * 0.02
                            textGap: row.width * 0.02
                            rightMargin: row.width * 0.02
                            marksMargin: row.width * 0.017
                            markCursor: wall.currentIndex
                            visibleStart: wall.firstVisibleIndex
                            visibleEnd: wall.lastVisibleIndex
                            markIndex: row.index
                            markAlbumId: row.albumId
                            markStarred: row.starred
                            markRating: row.rating
                            markRatingVisible: true
                            searchQuery: wall.searchQuery
                            searchCurrent: row.index === wall.searchCurrentRow
                        }
                    }
                }
            }
        }
    }
}
