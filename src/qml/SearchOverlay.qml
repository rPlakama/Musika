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
import musika
import "Theme.js" as Theme

// Floating global search (Shift Space F): one modal panel with Songs | Albums |
// Artists columns (RowLayout weights 38/40/22), filtered live from the library.
// Typing mode: keys type, Tab/arrows navigate, Esc -> normal mode.
// Normal mode: hjkl navigates, i re-enters typing, Enter opens/plays, Esc
// closes and returns focus to the previous region.
Item {
    id: root

    property bool opened: false
    property bool loading: false
    property bool typing: true // false = search "normal mode" (hjkl navigates)
    property int activeColumn: 0 // 0 songs, 1 albums, 2 artists (priority Songs > Albums > Artists)
    // UI-55: the item behind the floating panel whose pixels feed the acrylic
    // blur (the shell, or the fullscreen player when it is up).
    property Item backdrop: null
    // UI-49: last query that already got the denied cue, so one response (or
    // one Enter) lights it once and holding a zero query never strobes.
    property string _zeroQuery: ""
    /// The panel's stable height (chrome, row sizing and the panel itself).
    readonly property real panelRefHeight: root.height * 0.78
    /// Only fetch the rows a column can actually paint. Before the layout has
    /// a height (first open) ask for the old full screenful so the payload is
    /// not starved to one row.
    readonly property int resultLimit: {
        if (!songsColumn || songsColumn.height <= 0)
            return 20;
        return Math.max(1, songsColumn.visibleRowCount);
    }
    /// A live query with no hits in any of the three sections: the panel
    /// replaces the stack with one message.
    readonly property bool nothingFound: field.text.length > 0
        && !root.loading
        && songResults.count + albumResults.count + artistResults.count === 0

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)
    signal artistActivated(string artistId, string artistName, string artistCover)
    /// ARC-I: Alt-hold quick actions armed on the active column's cursor row;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    // Keep rendering while the close fade runs (the panel owns the opacity).
    visible: opened || panel.opacity > 0
    z: Theme.zOverlay

    // UI-49: denied-input cue for unmatched results. The bar itself never
    // moves (no shake): the cue is a red outline flash on the field.
    SequentialAnimation {
        id: modeFlash
        NumberAnimation {
            target: fieldFlash
            property: "opacity"
            to: 1
            duration: Theme.motion.flash
        }
        NumberAnimation {
            target: fieldFlash
            property: "opacity"
            to: 0
            duration: Theme.motion.quick
        }
    }

    function denied() {
        if (Theme.reduceMotion || field.text.length === 0)
            return;
        modeFlash.restart();
    }

    function open() {
        openWith("");
    }

    /// Search asks for exactly the rows the columns can display.
    function _requestSearch(text) {
        Backend.requestSearch(text, root.resultLimit);
    }

    /// Opens in typing mode with `text` already in the field. Paste (VI-21)
    /// uses this so the user sees the clipboard text and can correct it in
    /// place before choosing a result.
    function openWith(text) {
        opened = true;
        // Scope-switching keys (Ctrl-W) are unavailable while search is open.
        Backend.setSearchOpen(true);
        clearResults();
        activeColumn = 0;
        field.text = text;
        loading = true;
        // S-9: an empty field opens on the Most-played surface; a typed field
        // goes through the debounce.
        if (text.length === 0)
            root._requestSearch("");
        typing = true;
        field.forceActiveFocus();
        field.cursorPosition = field.text.length;
    }

    function close() {
        opened = false;
        typing = false;
        // VI-4: leaving the overlay clears every column's visual selection.
        songsColumn.clearSelection();
        albumsColumn.clearSelection();
        artistsColumn.clearSelection();
        Backend.setSearchOpen(false);
        Backend.releaseFocus();
    }

    // ESC in typing -> normal mode; ESC in normal -> leave to Main.
    function pressEscape() {
        if (typing) {
            typing = false;
            field.focus = false;
            Backend.releaseFocus();
        } else {
            close();
        }
    }

    function enterTyping() {
        // VI-4: going back to typing ends the normal-mode selection.
        columnFor(activeColumn).clearSelection();
        typing = true;
        field.forceActiveFocus();
        // Re-entry selects the whole query (qmllint cannot see TextInput's
        // methods through a custom base; runtime is fine).
        field.selectAll();
    }



    // Search normal mode: hjkl navigates, Enter opens/plays, i types again.
    function handleAction(action) {
        if (action === "back") {
            // VI-4: the first Esc ends the active column's visual selection.
            var column = columnFor(activeColumn);
            if (column.hasSelection)
                column.clearSelection();
            else
                close();
        } else if (action === "insert_mode") {
            enterTyping();
        } else if (action === "pane_cycle" || action === "pane_cycle_back") {
            // Tab / Shift-Tab cycle the sections in normal mode too (typing
            // mode routes them through SearchInput's own handlers).
            moveColumn(action === "pane_cycle" ? 1 : -1);
        } else if (action === "move_left" || action === "move_right") {
            moveColumn(action === "move_left" ? -1 : 1);
        } else if (action === "move_up") {
            moveRow(-1);
        } else if (action === "move_down") {
            moveRow(1);
        } else if (action === "top" || action === "bottom") {
            moveEdge(action === "bottom");
        } else if (action === "page_up" || action === "page_down") {
            columnFor(activeColumn).pageJump(action === "page_down");
        } else if (action.indexOf("extend_") === 0) {
            columnFor(activeColumn).extendSelection(
                "move_" + action.substring(7));
        } else if (action === "visual") {
            columnFor(activeColumn).toggleSelection();
        } else if (action === "visual_all") {
            columnFor(activeColumn).selectAll();
        } else if (action === "enter") {
            activate();
        }
    }

    function clearResults() {
        songResults.clear();
        albumResults.clear();
        artistResults.clear();
        songsColumn.cursor = -1;
        albumsColumn.cursor = -1;
        artistsColumn.cursor = -1;
        root._zeroQuery = "";
    }

    function columnFor(column) {
        if (column === 0)
            return songsColumn;
        if (column === 1)
            return albumsColumn;
        return artistsColumn;
    }

    /// ARC-UI-2: the overlay as one `/` surface — all three result columns,
    /// in column order; a jump also activates the column it lands in, and
    /// the session highlights follow the candidate's column.
    function _searchColumns() {
        return [songsColumn, albumsColumn, artistsColumn];
    }

    readonly property var searchCounts: [
        songsColumn.model ? songsColumn.model.count : 0,
        albumsColumn.model ? albumsColumn.model.count : 0,
        artistsColumn.model ? artistsColumn.model.count : 0
    ]

    property string searchQuery: ""
    property int searchCurrentRow: -1

    onSearchQueryChanged: _applySearch()
    onSearchCurrentRowChanged: _applySearch()
    onSearchCountsChanged: _applySearch()

    function searchRows() {
        var out = [];
        var cols = _searchColumns();
        var base = 0;
        for (var c = 0; c < cols.length; ++c) {
            var rows = cols[c].searchRows();
            for (var i = 0; i < rows.length; ++i) {
                if (rows[i] !== undefined)
                    out[base + i] = rows[i];
            }
            base += rows.length;
        }
        return out;
    }

    function searchJump(row) {
        var counts = root.searchCounts;
        var cols = _searchColumns();
        for (var c = 0; c < counts.length; ++c) {
            if (row < counts[c]) {
                root.activeColumn = c;
                cols[c].searchJump(row);
                return;
            }
            row -= counts[c];
        }
    }

    function _applySearch() {
        var counts = root.searchCounts;
        var cols = _searchColumns();
        var run = 0;
        for (var c = 0; c < cols.length; ++c) {
            var here = root.searchCurrentRow >= run
                && root.searchCurrentRow < run + counts[c];
            cols[c].searchQuery = root.searchQuery;
            cols[c].searchCurrentRow = here ? root.searchCurrentRow - run : -1;
            run += counts[c];
        }
    }

    /// ARC-I: Alt armed — the active column anchors the overlay on its cursor
    /// row with the recycled verb set for its kind (song/album/artist).
    function armQuickActions() {
        columnFor(activeColumn).armQuickActions();
    }

    function moveRow(delta) {
        var column = columnFor(activeColumn);
        if (!column.model || column.model.count === 0)
            return;
        var next = Math.max(0, Math.min(column.model.count - 1, column.cursor + delta));
        column.cursor = next;
    }

    // S-9: the "Most played" right-hand meta.
    function playCountLabel(count) {
        var n = Number(count) || 0;
        return n + (n === 1 ? " play" : " plays");
    }

    function moveColumn(delta) {
        // S-9: the empty-query surface has a single column.
        if (field.text.length === 0)
            return;
        // VI-4: one column's selection at a time.
        columnFor(activeColumn).clearSelection();
        activeColumn = (activeColumn + delta + 3) % 3;
        var column = columnFor(activeColumn);
        if (column.cursor < 0 && column.model && column.model.count > 0)
            column.cursor = 0;
    }

    // VI-11: `gg`/`G` on the active column only. The column list follows its
    // cursor through the `currentIndex` binding, so only the cursor moves.
    function moveEdge(last) {
        var column = columnFor(activeColumn);
        if (!column.model || column.model.count === 0)
            return;
        column.cursor = last ? column.model.count - 1 : 0;
        // Snap, don't scroll: a long edge jump must not play the follow
        // animation all the way across the column (the lists snap too).
        column.snapToCursor();
    }

    function activate() {
        var column = columnFor(activeColumn);
        var hit = column.model ? column.model.get(column.cursor) : null;
        if (!hit) {
            // UI-49: Enter on a query with no hits anywhere is denied input.
            if (songResults.count + albumResults.count + artistResults.count === 0)
                root.denied();
            return;
        }
        // VI-4: activating a row ends its column's visual selection.
        column.clearSelection();
        if (activeColumn === 0)
            root.songActivated(hit.id);
        else if (activeColumn === 1)
            root.albumActivated(hit.id, hit.title, hit.artist, hit.year, hit.tracks, hit.cover, hit.starred === true);
        else
            root.artistActivated(hit.id, hit.title, hit.cover);
        close();
    }

    function apply(payload) {
        if (!root.opened)
            return;

        var data = JSON.parse(payload);
        // A response for a query the field no longer holds is stale.
        if (data.query !== undefined && data.query !== field.text)
            return;

        // UI-41: results crossfade in on payload swaps instead of flashing.
        columnsRow.opacity = 0;
        songResults.clear();
        albumResults.clear();
        artistResults.clear();

        var i;
        for (i = 0; i < data.songs.length; i++) {
            var song = data.songs[i];
            songResults.append({
                id: song.id,
                title: song.title,
                subtitle: (song.artist ? song.artist : "") + (song.album ? " · " + song.album : ""),
                // S-9: the empty-query surface (Most played) shows the play
                // count where a search hit shows the duration.
                meta: data.most_played
                    ? root.playCountLabel(song.play_count)
                    : Theme.formatTime(song.duration),
                cover: song.cover,
                // ARC-I: the recycled Alt verbs key on these names.
                songId: song.id,
                albumId: song.album_id ? song.album_id : "",
                starred: song.starred === true,
                rating: 0
            });
        }
        for (i = 0; i < data.albums.length; i++) {
            var album = data.albums[i];
            albumResults.append({
                id: album.id,
                title: album.title,
                subtitle: album.artist ? album.artist : "",
                meta: album.year ? String(album.year) : "",
                cover: album.cover,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count),
                albumId: album.id,
                songId: "",
                starred: album.starred === true,
                rating: Number(album.rating) || 0
            });
        }
        for (i = 0; i < data.artists.length; i++) {
            var artist = data.artists[i];
            // Count stacks under the name (two-line cell), so the narrow
            // column needs no right-side meta gutter.
            artistResults.append({
                id: artist.id,
                title: artist.name,
                subtitle: artist.album_count + (artist.album_count === 1 ? " album" : " albums"),
                meta: "",
                cover: artist.cover,
                artistId: artist.id,
                songId: "",
                albumId: "",
                starred: artist.starred === true,
                rating: Number(artist.rating) || 0
            });
        }

        songsColumn.cursor = songResults.count > 0 ? 0 : -1;
        albumsColumn.cursor = albumResults.count > 0 ? 0 : -1;
        artistsColumn.cursor = artistResults.count > 0 ? 0 : -1;
        // The mark window follows the fresh result set.
        songsColumn.requestMarkWindow();
        // Lookup priority picks the column the cursor starts in; the
        // empty-query surface only has the Most-played column.
        activeColumn = data.most_played
            ? 0
            : (songResults.count > 0 ? 0 : (albumResults.count > 0 ? 1 : 2));

        loading = false;
        Qt.callLater(function () {
            columnsRow.opacity = 1;
        });

        // UI-49: the response landing on zero hits is denied input — once per
        // distinct query, never again while the same zero query is held. An
        // empty library is not denied input.
        var total = songResults.count + albumResults.count + artistResults.count;
        if (!data.most_played && total === 0) {
            if (field.text !== root._zeroQuery)
                root.denied();
            root._zeroQuery = field.text;
        } else {
            root._zeroQuery = "";
        }
    }

    ListModel {
        id: songResults
    }

    ListModel {
        id: albumResults
    }

    ListModel {
        id: artistResults
    }

    Timer {
        id: debounce
        interval: 120
        onTriggered: {
            root.loading = true;
            if (field.text.length === 0) {
                // S-9: clearing the field returns to the Most-played surface
                // (drop the typed hits first so only the surface swaps).
                root.clearResults();
                root._requestSearch("");
                return;
            }
            root._requestSearch(field.text);
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "search" && root.opened)
                root.apply(payload);
        }
    }

    // Dim backdrop (the one sanctioned scrim); click outside to dismiss.
    Rectangle {
        radius: Theme.radius
        anchors.fill: parent
        color: Palette.scrim
        opacity: root.opened ? Theme.scrimOpacity : 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }

        TapHandler {
            enabled: root.opened
            onTapped: root.close()
        }
    }

    AcrylicPanel {
        id: panel
        width: root.width * 0.84
        height: root.panelRefHeight
        x: (root.width - width) / 2
        // Slides up into place, back down on close (motion §3.1).
        y: (root.height - height) / 2 + (root.opened ? 0 : height * 0.04)
        // UI-55: the panel is the acrylic surface (Palette.surface tint over
        // the blurred backdrop); the field and chips keep the higher
        // container tones for contrast.
        tint: Palette.surface
        backdrop: root.backdrop
        radius: Theme.radius
        border.color: Palette.primary
        border.width: Math.max(1, root.height * Theme.hairlineRatio)
        opacity: root.opened ? 1 : 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }
        Behavior on y {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }
        ColumnLayout {
            anchors.fill: parent
            anchors.margins: root.panelRefHeight * 0.042
            spacing: root.panelRefHeight * 0.032

            // The search bar, full width: the old mode chip is gone — the
            // bar's own focus highlight is the INSERT cue.
            Rectangle {
                radius: Theme.radius
                id: searchRow
                Layout.fillWidth: true
                implicitHeight: root.panelRefHeight * 0.066
                color: "transparent"

                // ARC-UI-2: the shared search input (binds live in
                // `SearchInput`); only the context routing is set here.
                SearchInput {
                    id: field
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    leftPadding: root.panelRefHeight * 0.024
                    rightPadding: root.panelRefHeight * 0.024
                    color: Palette.onSurface
                    font.family: Theme.fonts.body
                    font.pixelSize: root.panelRefHeight * 0.030

                    onTextChanged: debounce.restart()

                    onStepNext: root.moveColumn(1)
                    onStepPrevious: root.moveColumn(-1)
                    onStepUp: root.moveRow(-1)
                    onStepDown: root.moveRow(1)
                    onSubmitted: root.activate()
                    onDismissed: root.pressEscape()
                }

                // The hint is ours, not the style's: Qt hides a centred
                // placeholder while the field is focused, and this one must
                // stay in INSERT too — it vanishes only when there is text.
                FadeText {
                    anchors.fill: field
                    visible: field.text.length === 0
                    text: "Search anything"
                    color: Palette.onSurfaceVariant
                    font.family: Theme.fonts.body
                    font.pixelSize: root.panelRefHeight * 0.030
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    animated: false
                }

                // UI-49: the denied cue — a red outline flash over the bar.
                Rectangle {
                    id: fieldFlash
                    anchors.fill: field
                    radius: Theme.radius
                    color: "transparent"
                    border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    border.color: Palette.error
                    opacity: 0
                }
            }

            // All three sections empty: one message in the panel itself
            // replaces the stack (and the panel shrinks to fit it).
            FadeText {
                Layout.fillWidth: true
                Layout.fillHeight: root.nothingFound
                visible: root.nothingFound || opacity > 0
                opacity: root.nothingFound ? 1 : 0
                text: "Nothing Found"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("title", root.panelRefHeight)
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                animated: false

                Behavior on opacity {
                    NumberAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.accelerate
                    }
                }
            }

            // The 0.3.0 modal: three result columns side by side. Widths are
            // RowLayout weights (38/40/22), not pixels: Albums gets the room
            // its titles need, Artists is narrow since it only carries a name
            // plus its count. The empty-query Songs surface takes the whole
            // row once the other two are hidden.
            RowLayout {
                id: columnsRow
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: !root.nothingFound
                spacing: root.panelRefHeight * 0.012

                Behavior on opacity {
                    NumberAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                SearchColumn {
                    id: songsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 38
                    Layout.fillHeight: true
                    // S-9: the empty-query surface uses this column alone.
                    title: field.text.length > 0 ? "Songs" : "Most played"
                    model: songResults
                    active: root.activeColumn === 0
                    loading: root.loading
                    hasQuery: field.text.length > 0
                    headerRefHeight: columnsRow.height
                    quickActionKind: "song"
                    selectionField: "songId"

                    onQuickAction: (target, actions) => root.quickAction(target, actions)

                    onPicked: index => {
                        songsColumn.cursor = index;
                        root.activeColumn = 0;
                        root.activate();
                    }
                }

                // Structural rule: keeps each column's header/count contained
                // instead of bleeding into its neighbour (UI-47).
                Rectangle {
                    radius: Theme.radius
                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(1, root.panelRefHeight * Theme.hairlineRatio)
                    color: Palette.outlineVariant
                    z: 1
                    visible: field.text.length > 0
                }

                SearchColumn {
                    id: albumsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 40
                    Layout.fillHeight: true
                    title: "Albums"
                    model: albumResults
                    active: root.activeColumn === 1
                    loading: root.loading
                    hasQuery: field.text.length > 0
                    visible: field.text.length > 0
                    headerRefHeight: columnsRow.height
                    quickActionKind: "album"
                    selectionField: "albumId"

                    onQuickAction: (target, actions) => root.quickAction(target, actions)

                    onPicked: index => {
                        albumsColumn.cursor = index;
                        root.activeColumn = 1;
                        root.activate();
                    }
                }

                Rectangle {
                    radius: Theme.radius
                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(1, root.panelRefHeight * Theme.hairlineRatio)
                    color: Palette.outlineVariant
                    z: 1
                    visible: field.text.length > 0
                }

                SearchColumn {
                    id: artistsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 22
                    Layout.fillHeight: true
                    title: "Artists"
                    roundedArt: true
                    model: artistResults
                    active: root.activeColumn === 2
                    loading: root.loading
                    hasQuery: field.text.length > 0
                    visible: field.text.length > 0
                    headerRefHeight: columnsRow.height
                    quickActionKind: "artist"
                    selectionField: "artistId"

                    onQuickAction: (target, actions) => root.quickAction(target, actions)

                    onPicked: index => {
                        artistsColumn.cursor = index;
                        root.activeColumn = 2;
                        root.activate();
                    }
                }
            }

            // Footer rule: separates the keybinds from the snapped row area.
            Rectangle {
                radius: Theme.radius
                Layout.fillWidth: true
                Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                color: Palette.outlineVariant
            }

            // One cheat sheet, per mode (no duplicate Esc hint).
            FadeText {
                Layout.fillWidth: true
                text: root.typing ? "Type to filter  ·  Tab/↑/↓ navigate  ·  Enter open/play  ·  Esc normal mode" : "Tab/h/l sections  ·  j/k rows  ·  i type  ·  Enter open/play  ·  Esc close"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.panelRefHeight)
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
