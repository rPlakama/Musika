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

// Artist entry (P-8, milestone A): round avatar + name over a four-column
// album grid with titles under the covers. Pushed from ArtistList and from
// artist hits in search; Enter on a cover opens the album page, Enter/Back
// from here always returns to Main (see Home.leaveArtistEntry). Favorite,
// stars, biography and the cache rule land in milestones B-D.
Rectangle {
    radius: Theme.radius
    id: root

    signal back
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)

    // ARC-43/ARC-UI-1: Alt-hold quick actions for an album card — four fixed
    // verbs, minus Go to Album (Enter already opens it): that slot stays the
    // open "—".
    signal quickAction(var target, var actions)

    /// ARC-UI-3: the album wall rides the same panel as the Main lists
    /// (`ScrollPanel`, SCROLLGRIDSTUDY.md) in grid mode; `Shift-C` swaps it
    /// to a compact list and every shared goodie (gg/G, the `/` contract,
    /// selection, memory, adaptive states) comes with it. The wall lives in
    /// `wall` below.
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

    /// ARC-43/ARC-UI-1: Alt armed the gesture — anchor on the keyboard cursor
    /// card with the recycled album verb set; a visual selection swaps in the
    /// batch verb set (VI-4).
    function armQuickActions() {
        // UI-60: a scoped header chip owns the gesture. The favourite chip
        // rates the artist itself (the same south slot the row marks use);
        // without that, Alt fell through to the album under the grid cursor.
        if (root.chipScope === 2) {
            var artist = {
                artistId: root.artistId,
                starred: root.artistStarred,
                rating: root.artistRating
            };
            var artistActions = verbs.artist(artist, function(level) {
                root.artistRating = level;
            });
            var wantStar = artistActions.south.favorite === true;
            var toggleStar = artistActions.south.invoke;
            artistActions.south.invoke = function() {
                toggleStar();
                root.artistStarred = wantStar;
            };
            quickAction(favoriteChip, artistActions);
            return;
        }
        if (albums.count === 0 || wall.currentIndex < 0)
            return;
        var cell = wall.view.itemAtIndex(wall.currentIndex);
        if (!cell)
            return;
        var albumItem = albums.get(wall.currentIndex);
        var albumPreview = function(level) {
            albums.setProperty(wall.currentIndex, "rating", level);
        };
        var ids = wall.hasSelection
            ? wall.selectedIds("albumId")
            : [albumItem.albumId];
        var actions = wall.hasSelection
            ? verbs.forSelection("album", ids, wall.selectedStarred("albumId"))
            : verbs.album(albumItem,
                Backend.cachedAlbumIds.indexOf(albumItem.albumId) !== -1, albumPreview);
        // ARC-I-4: the decoration rides the verb so the mark tracks the
        // action immediately — `south.favorite` is the state the verb leaves
        // every selected row in (the server call itself is unchanged).
        var want = actions.south.favorite === true;
        var star = actions.south.invoke;
        actions.south.invoke = function() {
            star();
            wall.markStarred(want);
        };
        quickAction(cell, actions);
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
    // UI-54: per-artist scroll memory for the album grid.
    readonly property string memoryKey: "artist:" + root.artistId
    // Home uses this to send `Esc` straight to Main (P-8) instead of popping
    // back to the Artists segment or a search result.
    readonly property bool exitsToMain: true

    // ARC-UI-3: the grid columns; the scroll body derives the cell size.
    readonly property int columns: 4
    readonly property real avatarSide: Math.max(96, Math.min(height * 0.22, 256))

    // Study §2 "name+server tag": host only, never the full URL.
    readonly property string serverTag: {
        var url = Backend.serverURL;
        var mark = url.indexOf("://");
        var rest = mark >= 0 ? url.substring(mark + 3) : url;
        var slash = rest.indexOf("/");
        return slash >= 0 ? rest.substring(0, slash) : rest;
    }

    color: Palette.surface

    // A color with a replaced alpha, for the ambience gradient stops (same
    // helper AlbumEntry/NowPlaying carry).
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }


    ListModel {
        id: albums
    }

    QuickActions {
        id: verbs
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
        if (artist.cover && artist.cover.length > 0)
            root.artistCover = artist.cover;
        var count = Number(artist.album_count);
        root.artistMeta = count + (count === 1 ? " album" : " albums");

        albums.clear();
        var rows = artist.albums ? artist.albums : [];
        for (var i = 0; i < rows.length; i++) {
            var album = rows[i];
            var meta = [];
            if (album.year)
                meta.push(String(album.year));
            meta.push(album.song_count + (album.song_count === 1 ? " song" : " songs"));
            albums.append({
                albumId: album.id,
                title: album.title,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count),
                meta: meta.join("   ·   "),
                cover: album.cover ? album.cover : "",
                starred: album.starred === true,
                rating: Number(album.rating) || 0
            });
        }
        if (albums.count > 0 && (wall.currentIndex < 0 || wall.currentIndex >= albums.count))
            wall.currentIndex = 0;
        // UI-54: reopening the entry restores the cursor + offset (and the
        // ARC-UI-3 layout); later reloads keep the live state.
        wall.restore();
        root.requestMarkWindow();
    }

    /// The state marks (favourite hearts, download glyphs) render only
    /// inside the wall cursor's ±15-row window — same rule as the segments.
    function markVisible(index) {
        return Math.abs(index - Math.max(0, wall.currentIndex)) <= 15;
    }

    /// Ask the backend for the download state of that window only: album
    /// cards are cached when every track of them is finished.
    function requestMarkWindow(force) {
        var albumsWindow = [];
        var at = Math.max(0, wall.currentIndex);
        var lo = Math.max(0, at - 15);
        var hi = Math.min(albums.count - 1, at + 15);
        for (var i = lo; i <= hi; ++i) {
            var album = albums.get(i);
            if (album && album.albumId)
                albumsWindow.push(album.albumId);
        }
        if (force)
            Backend.refreshDownloads("[]", JSON.stringify(albumsWindow));
        else
            Backend.requestDownloads("[]", JSON.stringify(albumsWindow));
    }

    Connections {
        target: wall
        function onCurrentIndexChanged() { root.requestMarkWindow(); }
    }

    function activateCurrent() {
        var album = albums.get(wall.currentIndex);
        if (album)
            root.albumActivated(album.albumId, album.title, album.artist, album.year, album.tracks, album.cover, album.starred === true);
    }

    /// Tab scope over the header chips (same ring as the status bar): 0
    /// download, 1 shuffle, 2 favorite. -1 returns Enter to the album grid;
    /// any grid move drops the ring back there.
    property int chipScope: -1

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

    /// The shuffle chip: a time-seeded album rotation of this artist,
    /// queued and played from its head (the backend's seeded walk).
    function shuffle() {
        if (root.artistId.length > 0)
            Backend.shuffleArtist(root.artistId);
    }

    Component.onCompleted: reload()

    // The shared panel owns movement (grid walks the wall by columns, list
    // steps ±1, gg/G snap through `edgeJump`); this only tracks the header
    // chip scope and the Home edge contract: false lets `h` leave for the
    // sidebar from the leftmost column.
    function moveWithinGrid(action) {
        if (!wall.moveWithinGrid(action))
            return false;
        // Moving the wall ends the header chip scope (Enter opens albums).
        root.chipScope = -1;
        return true;
    }

    // Chip scope and Enter-on-chip are the host's; everything else goes to
    // the shared panel (movement, gg/G, Enter on a card, layout_toggle).
    // `Esc` is handled by Home (leaves the entry for Main regardless of
    // where it was opened from).
    function handleAction(action) {
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            // Tab cycles the header chips (Download → Shuffle → Favorite);
            // Shift+Tab walks the same circle backwards.
            var step = action === "pane_cycle" ? 1 : 2;
            root.chipScope = (root.chipScope + step) % 3;
            return;
        }
        if (action === "enter" && root.chipScope >= 0) {
            root.activateChip();
            return;
        }
        if (moveWithinGrid(action))
            return;
        wall.handleAction(action);
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
        }
        // A fetch finished: re-read this wall's own window (forced).
        function onRowReload() {
            root.requestMarkWindow(true);
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onAlbum_order_changed() {
            root.reload();
        }
        // UI-60: patch the header rating or the matching album card.
        function onRatingChanged(kind, id, rating) {
            if (kind === "artist" && id === root.artistId) {
                root.artistRating = rating;
                return;
            }
            if (kind !== "album")
                return;
            for (var i = 0; i < albums.count; ++i) {
                if (albums.get(i).albumId === id)
                    albums.setProperty(i, "rating", rating);
            }
        }
    }

    // Ambience (D-5): same rule as AlbumEntry/NowPlaying — the artist art
    // through `CoverImage.ambient`, blurred and washed into the surface by the
    // gradient; the one non-modal blur (content ambience, never elevation).
    CoverImage {
        id: coverBackdrop
        anchors.fill: parent
        source: root.artistCover
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
                onClicked: root.back()
            }

            Item {
                Layout.fillWidth: true
            }
        }

        // UI-53 header standard: one major rectangle holds three transparent
        // containers — left identity (30% width: round avatar + name + album
        // count) and a right stack (the remaining width) split 70% height for
        // the biography over 30% for the action row. Fixed anchors, so a long
        // biography can never squeeze the identity or grow the header.
        Rectangle {
            radius: Theme.radius
            id: header
            Layout.fillWidth: true
            Layout.preferredHeight: root.avatarSide
            color: "transparent"

            Rectangle {
                radius: Theme.radius
                id: identityBox
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                width: parent.width * 0.3
                color: "transparent"

                // The only circle in the product is the artist avatar (R3).
                RowLayout {
                    anchors.fill: parent
                    spacing: root.width * 0.015

                    Rectangle {
                        readonly property real side: Math.min(root.avatarSide,
                            identityBox.width * 0.5)

                        Layout.preferredWidth: side
                        Layout.preferredHeight: side
                        Layout.alignment: Qt.AlignVCenter
                        radius: width * 0.5
                        color: Palette.primaryContainer
                        clip: true

                        CoverImage {
                            anchors.fill: parent
                            source: root.artistCover
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignVCenter
                        spacing: root.height * 0.006

                        FadeText {
                            Layout.fillWidth: true
                            text: root.artistName
                            color: Palette.onSurface
                            font.pixelSize: root.height * 0.036
                            font.bold: true
                            elide: Text.ElideRight
                        }
                        FadeText {
                            Layout.fillWidth: true
                            text: root.artistMeta + (root.serverTag.length > 0 ? "   ·   " + root.serverTag : "")
                            color: Palette.onSurfaceVariant
                            font.pixelSize: root.height * 0.02
                            elide: Text.ElideRight
                        }
                    }
                }
            }

            Rectangle {
                radius: Theme.radius
                id: descriptionBox
                anchors.left: identityBox.right
                anchors.right: parent.right
                anchors.top: parent.top
                height: parent.height * 0.7
                color: "transparent"
                visible: !Theme.isNarrow(root.width)

                // 15 px metrics: the capacity reference for the size ramp.
                FontMetrics {
                    id: bioMetrics
                    font.family: Theme.fonts.body
                    font.pixelSize: 15
                }

                FadeText {
                    id: descriptionLabel
                    anchors.fill: parent
                    anchors.leftMargin: root.width * 0.015

                    // UI-52/UI-53: font size ramps 15..17 by how much of this
                    // rectangle the text fills — `capacity` is how many
                    // characters fit at the 15 px floor (font metrics × the
                    // lines that fit at that floor); a short biography grows
                    // toward 17, one that exactly fills the box sits at 15,
                    // overflow stays 15 and elides.
                    readonly property int lines: Math.max(1,
                        Math.floor(height / (15 * 1.3)))
                    readonly property real charsPerLine: Math.max(1,
                        Math.floor(width / bioMetrics.averageCharacterWidth))
                    readonly property real capacity: charsPerLine * lines

                    text: root.descriptionText
                    color: Palette.onSurfaceVariant
                    font.pixelSize: Theme.clamp(
                        15 + (1 - text.length / capacity) * 2, 15, 17)
                    wrapMode: Text.WordWrap
                    maximumLineCount: lines
                    elide: Text.ElideRight
                }
            }

            Rectangle {
                radius: Theme.radius
                id: actionsBox
                anchors.left: identityBox.right
                anchors.right: parent.right
                anchors.top: descriptionBox.bottom
                anchors.bottom: parent.bottom
                color: "transparent"
                visible: !Theme.isNarrow(root.width)

                // UI-60: the five-star array leads the strip (hover preview,
                // click sets/clears); the favourite chip keeps the count
                // badge on the right.
                Row {
                    anchors.left: parent.left
                    anchors.leftMargin: root.width * 0.015
                    anchors.verticalCenter: parent.verticalCenter

                    StarRating {
                        anchors.verticalCenter: parent.verticalCenter
                        unit: root.height * 0.05
                        rating: root.artistRating
                        onPicked: (level) => {
                            root.artistRating = level;
                            Backend.rateArtist(root.artistId, level);
                        }
                    }
                }

                Row {
                    anchors.right: parent.right
                    anchors.rightMargin: root.width * 0.015
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: root.height * 0.010

                    ActionChip {
                        unit: root.height * 0.06
                        // CACHE-5/8: artist container download toggle. ON
                        // marks and fetches every song of the artist, OFF
                        // drops the rule, its marks and its rule-owned blobs;
                        // the `artist_cache` payload reconciles (including
                        // `active`). The visual is the shared
                        // ARC-UI-6 DownloadButton. Counts label is CACHE-7.
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

                        // UI-60: the shared favourite button carries the
                        // artist's state + count (the Alt overlay's rating
                        // step reconciles through `ratingChanged`).
                        FavoriteButton {
                            anchors.fill: parent
                            variant: "chip"
                            unit: parent.height * 0.42
                            starred: root.artistStarred
                            rating: root.artistRating
                        }

                        ScopeRing {
                            shown: root.chipScope === 2
                        }
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

        // Albums: the shared panel (ARC-UI-3, SCROLLGRIDSTUDY.md) — the same
        // `ScrollPanel` the Main list segments use, in grid mode: four covers
        // per row with the title and year/count underneath, gg/G, the `/`
        // contract, selection, memory and the empty/loading states. Shift-C
        // swaps it to a compact list.
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
                headerRatio: 0
                model: albums
                layouts: ["grid", "list"]
                layout: "list"
                columns: root.columns
                cellHeight: wall.cellWidth * 0.78 + wall.height * 0.085
                selectable: true
                memoryKey: root.memoryKey
                searchFields: ["title", "meta"]
                emptyText: "No albums indexed yet"

                onItemActivated: (index) => root.activateCurrent()

                cellDelegate: Component {
                    Rectangle {
                        radius: Theme.radius
                        id: cell
                        required property int index
                        required property string albumId
                        required property string title
                        required property string artist
                        required property string year
                        required property string tracks
                        required property string meta
                        required property string cover
                        required property bool starred
                        /// UI-60: 0..=5 album user rating.
                        required property int rating

                        readonly property bool current: GridView.isCurrentItem
                        // UI-50: cursor or hover expands the cover + title,
                        // the same selection stage the lists and search use.
                        readonly property bool emphasized: (cell.current || cellHover.hovered) && !Theme.reduceMotion

                        width: GridView.view.cellWidth
                        height: GridView.view.cellHeight
                        color: "transparent"

                        // VI-4: a card inside the visual selection wears the
                        // cursor fill.
                        readonly property bool selectedCard: wall.isSelected(cell.index)

                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: cell.width * 0.03
                            radius: Theme.radius
                            color: (cell.current || cell.selectedCard)
                                ? Palette.secondaryContainer
                                : (cellHover.hovered ? Palette.surfaceContainerHigh : "transparent")

                            Behavior on color {
                                ColorAnimation {
                                    duration: Theme.motion.quick
                                    easing.type: Easing.Bezier
                                    easing.bezierCurve: Theme.ease.standard
                                }
                            }
                        }

                        Rectangle {
                            id: coverBox
                            width: cell.width * 0.78
                            height: width
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.top: parent.top
                            anchors.topMargin: cell.height * 0.05
                            radius: Theme.radius
                            color: Palette.surfaceContainerLowest
                            border.color: cell.current ? Palette.primary : "transparent"
                            border.width: Math.max(1, cell.height * Theme.hairlineRatio)
                            clip: true

                            transformOrigin: Item.Center
                            scale: cell.emphasized ? Theme.selectionEmphasis : 1.0

                            Behavior on scale {
                                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                            }

                            CoverImage {
                                anchors.fill: parent
                                source: cell.cover
                                // ARC-40: ×2 headroom for the UI-50 hover scale.
                                sizeFactor: 2
                            }

                            // UI-56/ARC-I-2/UI-60: a favourited album carries
                            // its mark inside the cover's bottom-right corner
                            // with the rating count at its corner; the
                            // download glyph (every track cached) mirrors it
                            // on the other side — the same state marks the
                            // segments show.
                            Item {
                                visible: root.markVisible(cell.index)
                                    && (cell.starred || cell.rating > 0)
                                anchors.right: parent.right
                                anchors.bottom: parent.bottom
                                anchors.margins: parent.width * 0.06
                                width: parent.width * 0.085
                                height: width

                                Icon {
                                    anchors.fill: parent
                                    visible: cell.starred
                                    name: "baseline-favorite"
                                    symbolColor: Theme.iconStrong
                                }

                                RatingBadge {
                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    width: parent.width * 0.84
                                    height: width
                                    rating: cell.rating
                                }
                            }

                            Icon {
                                visible: root.markVisible(cell.index)
                                    && Backend.cachedAlbumIds.indexOf(cell.albumId) !== -1
                                anchors.left: parent.left
                                anchors.bottom: parent.bottom
                                anchors.margins: parent.width * 0.06
                                width: parent.width * 0.085 * 1.2
                                height: width
                                name: "baseline-download"
                                symbolColor: Palette.tertiary
                            }
                        }

                        SearchText {
                            id: titleText
                            anchors.top: coverBox.bottom
                            anchors.topMargin: cell.height * 0.025
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: parent.width * 0.86
                            text: cell.title
                            color: Palette.onSurface
                            horizontalAlignment: Text.AlignHCenter
                            query: wall.searchQuery
                            currentMatch: cell.index === wall.searchCurrentRow
                            elide: Text.ElideRight
                            font.pixelSize: Math.max(10, cell.height * 0.0525)
                            animated: false
                            transformOrigin: Item.Center
                            scale: cell.emphasized
                                ? Theme.fitScale(titleText.width, titleText.paintedWidth, Theme.selectionEmphasis)
                                : 1.0

                            Behavior on scale {
                                NumberAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                            }
                        }

                        SearchText {
                            id: metaText
                            anchors.top: titleText.bottom
                            anchors.topMargin: cell.height * 0.015
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: parent.width * 0.86
                            text: cell.meta
                            color: Palette.onSurfaceVariant
                            horizontalAlignment: Text.AlignHCenter
                            elide: Text.ElideRight
                            font.pixelSize: Math.max(9, cell.height * 0.04)
                            animated: false
                            query: wall.searchQuery
                            currentMatch: cell.index === wall.searchCurrentRow
                        }

                        HoverHandler {
                            id: cellHover
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            onTapped: {
                                wall.currentIndex = cell.index;
                                root.activateCurrent();
                            }
                        }
                    }
                }

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
                        color: (row.current || row.selectedRow)
                            ? Palette.secondaryContainer
                            : (rowHover.hovered ? Palette.surfaceContainerHigh : "transparent")

                        Behavior on color {
                            ColorAnimation {
                                duration: Theme.motion.quick
                                easing.type: Easing.Bezier
                                easing.bezierCurve: Theme.ease.standard
                            }
                        }

                        Rectangle {
                            id: rowCover
                            anchors.left: parent.left
                            anchors.leftMargin: row.width * 0.02
                            anchors.verticalCenter: parent.verticalCenter
                            width: row.height * 0.78
                            height: width
                            radius: Theme.radius
                            color: Palette.surfaceContainerLowest
                            clip: true

                            CoverImage {
                                anchors.fill: parent
                                source: row.cover
                                sizeFactor: 2
                            }
                        }

                        // REC-UI-1: the shared state-mark block on the right
                        // rail (only the grid cards keep cover corners).
                        RowMarks {
                            id: rowMarks
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.right: parent.right
                            anchors.rightMargin: row.width * 0.017
                            cursor: wall.currentIndex
                            index: row.index
                            albumId: row.albumId
                            starred: row.starred
                            rating: row.rating
                            showRating: true
                            base: row.height * 0.15
                        }

                        Column {
                            anchors.left: rowCover.right
                            anchors.leftMargin: row.width * 0.02
                            anchors.right: parent.right
                            // The marks own the right rail; the text elides
                            // before them (0 width when nothing is marked).
                            anchors.rightMargin: row.width * 0.02 + rowMarks.width
                                + (rowMarks.width > 0 ? row.width * 0.01 : 0)
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: row.height * 0.04

                            SearchText {
                                width: parent.width
                                text: row.title
                                color: Palette.onSurface
                                elide: Text.ElideRight
                                font.pixelSize: Theme.clamp(row.height * 0.26, 12, 18)
                                animated: false
                                query: wall.searchQuery
                                currentMatch: row.index === wall.searchCurrentRow
                            }

                            SearchText {
                                width: parent.width
                                text: row.meta
                                color: Palette.onSurfaceVariant
                                elide: Text.ElideRight
                                font.pixelSize: Theme.clamp(row.height * 0.19, 10, 14)
                                animated: false
                                query: wall.searchQuery
                                currentMatch: row.index === wall.searchCurrentRow
                            }
                        }

                        HoverHandler {
                            id: rowHover
                            cursorShape: Qt.PointingHandCursor
                        }

                        TapHandler {
                            onTapped: {
                                wall.currentIndex = row.index;
                                root.activateCurrent();
                            }
                        }
                    }
                }
            }
        }
    }
}
