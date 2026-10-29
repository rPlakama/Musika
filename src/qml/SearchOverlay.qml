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

// Floating global search (Shift Space F): one unified result list with a
// scope strip (Songs | Albums | Artists), filtered live from the library.
// Shift-H/Shift-L (and Tab/h/l) switch the result scope; j/k walks rows.
// Typing mode: keys type, Tab/arrows navigate, Esc -> normal mode.
// Normal mode: hjkl navigates, i re-enters typing, Enter opens/plays, Esc
// closes and returns focus to the previous region.
Item {
    id: root

    property bool opened: false
    property bool loading: false
    property bool typing: true // false = search "normal mode" (hjkl navigates)
    // ARC-43: Alt is a temporary retraction of typing; release restores it.
    property bool _altWasTyping: false
    // Unified result scope: 0 songs, 1 albums, 2 artists. Songs lead by
    // default; a scope with no hits falls through to the next one.
    property int scope: 0
    // Per-scope cursor memory, so switching scope returns to the last row.
    property var scopeCursors: [0, 0, 0]
    // UI-55: the item behind the floating panel whose pixels feed the acrylic
    // blur (the shell, or the fullscreen player when it is up).
    property Item backdrop: null
    // UI-49: last query that already got the denied cue, so one response (or
    // one Enter) lights it once and holding a zero query never strobes.
    property string _zeroQuery: ""
    readonly property var scopeTitles: ["Songs", "Albums", "Artists"]
    /// The panel's stable height (chrome, row sizing and the panel itself).
    readonly property real panelRefHeight: root.height * 0.78
    /// Only fetch the rows the list can actually paint. Before the layout has
    /// a height (first open) ask for the old full screenful so the payload is
    /// not starved to one row.
    readonly property int resultLimit: {
        if (!resultsColumn || resultsColumn.height <= 0)
            return 20;
        return Math.max(1, resultsColumn.visibleRowCount);
    }
    /// A live query with no hits in any scope: the panel replaces the list
    /// with one message.
    readonly property bool nothingFound: field.text.length > 0
        && !root.loading
        && songResults.count + albumResults.count + artistResults.count === 0

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover, bool starred)
    signal artistActivated(string artistId, string artistName, string artistCover)
    /// ARC-I: Alt-hold quick actions armed on the active cursor row; Home
    /// routes it to the shared overlay.
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

    /// Search asks for exactly the rows the unified list can display.
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
        scope = 0;
        field.text = text;
        loading = true;
        // S-9: an empty field opens on the scope's browse surface; a typed
        // field goes through the debounce.
        if (text.length === 0)
            root._requestSearch("");
        typing = true;
        field.forceActiveFocus();
        field.cursorPosition = field.text.length;
    }

    function close() {
        opened = false;
        typing = false;
        _altWasTyping = false;
        // VI-4: leaving the overlay clears the list's visual selection.
        resultsColumn.clearSelection();
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

    /// Alt armed: leave typing for normal-mode navigation without closing
    /// or clearing the query.
    function retractForQuickActions() {
        if (!opened)
            return;
        _altWasTyping = typing;
        if (typing) {
            typing = false;
            field.focus = false;
            Backend.releaseFocus();
        }
    }

    /// Alt released: return to typing only when that is where we started.
    function restoreAfterQuickActions() {
        if (!opened || !_altWasTyping)
            return;
        _altWasTyping = false;
        enterTyping();
    }

    function enterTyping() {
        // VI-4: going back to typing ends the normal-mode selection.
        resultsColumn.clearSelection();
        typing = true;
        field.forceActiveFocus();
        // Re-entry selects the whole query (qmllint cannot see TextInput's
        // methods through a custom base; runtime is fine).
        field.selectAll();
    }

    // Search normal mode: hjkl navigates, Enter opens/plays, i types again.
    function handleAction(action) {
        if (action === "back" || action === "cancel") {
            // VI-4: the first Esc ends the active visual selection.
            if (resultsColumn.hasSelection)
                resultsColumn.clearSelection();
            else
                close();
        } else if (action === "insert_mode") {
            enterTyping();
        } else if (action === "order_next" || action === "order_prev"
                || action === "scope_next" || action === "scope_prev") {
            // The library order keys and the unified scope keys both walk
            // search scopes here.
            var forward = action === "order_next" || action === "scope_next";
            moveScope(forward ? 1 : -1);
        } else if (action === "pane_cycle" || action === "pane_cycle_back") {
            // Tab / Shift-Tab cycle the scopes in normal mode too (typing
            // mode routes them through SearchInput's own handlers).
            moveScope(action === "pane_cycle" ? 1 : -1);
        } else if (action === "move_left" || action === "move_right") {
            moveScope(action === "move_left" ? -1 : 1);
        } else if (action === "move_up") {
            moveRow(-1);
        } else if (action === "move_down") {
            moveRow(1);
        } else if (action === "top" || action === "bottom") {
            moveEdge(action === "bottom");
        } else if (action === "page_up" || action === "page_down") {
            resultsColumn.pageJump(action === "page_down");
        } else if (action.indexOf("extend_") === 0) {
            resultsColumn.extendSelection("move_" + action.substring(7));
        } else if (action === "visual") {
            resultsColumn.toggleSelection();
        } else if (action === "visual_all") {
            resultsColumn.selectAll();
        } else if (action === "yank" || action === "yank_row") {
            // VI-8a: the active result column feeds the universal clipboard.
            resultsColumn.yankRows(action === "yank_row");
        } else if (action === "enter") {
            activate();
        }
    }

    function clearResults() {
        songResults.clear();
        albumResults.clear();
        artistResults.clear();
        resultsColumn.cursor = -1;
        root.scopeCursors = [0, 0, 0];
        root._zeroQuery = "";
    }

    function scopeModel(index) {
        if (index === 0)
            return songResults;
        if (index === 1)
            return albumResults;
        return artistResults;
    }

    function scopeCount(index) {
        var model = root.scopeModel(index);
        return model ? model.count : 0;
    }

    function setScope(index) {
        if (index === root.scope)
            return;
        root.scopeCursors[root.scope] = resultsColumn.cursor;
        root.scope = index;
        var count = root.scopeCount(index);
        var saved = root.scopeCursors[index];
        resultsColumn.cursor = count > 0
            ? Math.max(0, Math.min(count - 1, saved))
            : -1;
    }

    function moveScope(delta) {
        // VI-4: one scope's selection at a time.
        resultsColumn.clearSelection();
        setScope((root.scope + delta + 3) % 3);
    }

    /// ARC-UI-2: the overlay as one `/` surface — the visible scope's rows.
    function searchRows() {
        return resultsColumn.searchRows();
    }

    function searchJump(row) {
        resultsColumn.searchJump(row);
    }

    property string searchQuery: ""
    property int searchCurrentRow: -1

    onSearchQueryChanged: _applySearch()
    onSearchCurrentRowChanged: _applySearch()

    function _applySearch() {
        resultsColumn.searchQuery = root.searchQuery;
        resultsColumn.searchCurrentRow = root.searchCurrentRow;
    }

    /// ARC-I: Alt armed — the active scope anchors the overlay on its cursor
    /// row with the recycled verb set for its kind (song/album/artist).
    function armQuickActions() {
        resultsColumn.armQuickActions();
    }

    function moveRow(delta) {
        var model = root.scopeModel(root.scope);
        if (!model || model.count === 0)
            return;
        var next = Math.max(0, Math.min(model.count - 1, resultsColumn.cursor + delta));
        resultsColumn.cursor = next;
    }

    // S-9: the "Most played" right-hand meta.
    function playCountLabel(count) {
        var n = Number(count) || 0;
        return n + (n === 1 ? " play" : " plays");
    }

    // VI-11: `gg`/`G` on the unified list. The list follows its cursor
    // through the `currentIndex` binding, so only the cursor moves.
    function moveEdge(last) {
        var model = root.scopeModel(root.scope);
        if (!model || model.count === 0)
            return;
        resultsColumn.cursor = last ? model.count - 1 : 0;
        // Snap, don't scroll: a long edge jump must not play the follow
        // animation all the way across the list (the lists snap too).
        resultsColumn.snapToCursor();
    }

    function activate() {
        var model = root.scopeModel(root.scope);
        var hit = model ? model.get(resultsColumn.cursor) : null;
        if (!hit) {
            // UI-49: Enter on a query with no hits anywhere is denied input.
            if (songResults.count + albumResults.count + artistResults.count === 0)
                root.denied();
            return;
        }
        // VI-4: activating a row ends the visual selection.
        resultsColumn.clearSelection();
        if (root.scope === 0)
            root.songActivated(hit.id);
        else if (root.scope === 1)
            root.albumActivated(hit.id, hit.title, hit.artist, hit.year, hit.tracks, hit.cover, hit.starred === true);
        else
            root.artistActivated(hit.id, hit.title, hit.cover);
        close();
    }

    function rowEqual(current, next) {
        for (var key in next) {
            if (current[key] !== next[key])
                return false;
        }
        return true;
    }

    function sameRows(model, rows) {
        if (!model || model.count !== rows.length)
            return false;
        for (var i = 0; i < rows.length; ++i) {
            if (!root.rowEqual(model.get(i), rows[i]))
                return false;
        }
        return true;
    }

    function sameIds(model, rows) {
        if (!model || model.count !== rows.length)
            return false;
        for (var i = 0; i < rows.length; ++i) {
            if (model.get(i).id !== rows[i].id)
                return false;
        }
        return true;
    }

    /// Reconcile the model by identity, not by rebuilding it. Existing rows
    /// are moved to their new index (ListView.displaced animates that move),
    /// unchanged rows are left alone, and only genuinely new/removed rows are
    /// inserted/removed.
    function syncRows(model, rows) {
        for (var i = 0; i < rows.length; ++i) {
            var from = -1;
            for (var j = i; j < model.count; ++j) {
                if (model.get(j).id === rows[i].id) {
                    from = j;
                    break;
                }
            }
            if (from < 0) {
                model.insert(i, rows[i]);
                continue;
            }
            if (from !== i)
                model.move(from, i, 1);
            var current = model.get(i);
            if (!root.rowEqual(current, rows[i])) {
                for (var key in rows[i])
                    model.setProperty(i, key, rows[i][key]);
            }
        }
        while (model.count > rows.length)
            model.remove(model.count - 1);
    }

    function apply(payload) {
        if (!root.opened)
            return;

        var data = JSON.parse(payload);
        // A response for a query the field no longer holds is stale.
        if (data.query !== undefined && data.query !== field.text)
            return;

        // A repeated response with the same rows must not churn the models:
        // clearing/repopulating the ListModels reloads the top delegate and
        // makes an identical result flicker.
        if (root.sameRows(songResults, data.songs)
                && root.sameRows(albumResults, data.albums)
                && root.sameRows(artistResults, data.artists)) {
            loading = false;
            return;
        }

        // UI-41: results update in place; unchanged rows keep their delegates.
        var songRows = [];
        var albumRows = [];
        var artistRows = [];

        var i;
        for (i = 0; i < data.songs.length; i++) {
            var song = data.songs[i];
            songRows.push({
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
            albumRows.push({
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
            // Count stacks under the name (two-line cell).
            artistRows.push({
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

        var idsSame = root.sameIds(songResults, data.songs)
            && root.sameIds(albumResults, data.albums)
            && root.sameIds(artistResults, data.artists);
        // Drop the cursor before rows disappear: ListView otherwise cancels a
        // pending delegate whose old index is past the shrunken model (the
        // `DelegateModel::cancel: index out of range` warning).
        if (!idsSame)
            resultsColumn.cursor = -1;
        root.syncRows(songResults, songRows);
        root.syncRows(albumResults, albumRows);
        root.syncRows(artistResults, artistRows);
        resultsColumn.opacity = 1;

        // Same IDs mean this is an in-place metadata refresh, not a new
        // result set: keep every scope cursor and the active selection.
        if (!idsSame) {
            // Fresh hits reset every scope's cursor; the active scope starts
            // on its first row. An empty scope falls through to the next one,
            // so a query with only song hits lands there instead of showing a
            // blank artist list.
            root.scopeCursors = [0, 0, 0];
            if (root.scopeCount(root.scope) === 0) {
                for (var step = 0; step < 3; ++step) {
                    var candidate = (root.scope + step) % 3;
                    if (root.scopeCount(candidate) > 0) {
                        root.scope = candidate;
                        break;
                    }
                }
            }
            resultsColumn.cursor = root.scopeCount(root.scope) > 0 ? 0 : -1;
        }
        // The mark window follows the result set.
        resultsColumn.requestMarkWindow();

        loading = false;

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
                // S-9: clearing the field returns to the scope's browse
                // surface (drop the typed hits first so only the surface swaps).
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
        // A finished sync changes the library under an open overlay: re-ask
        // for the current query instead of leaving pre-sync empty results.
        function onLibrary_changed() {
            if (root.opened)
                root._requestSearch(field.text);
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
        width: root.width * 0.714
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

            // The search bar, full width: the bar's own focus highlight is
            // the INSERT cue.
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

                    onTextChanged: {
                        // Keep the match highlighter on the same text as the
                        // debounced request, not only the / command line binding.
                        resultsColumn.searchQuery = field.text;
                        resultsColumn.searchCurrentRow = -1;
                        debounce.restart();
                    }

                    onStepNext: root.moveScope(1)
                    onStepPrevious: root.moveScope(-1)
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
                    text: "Search artists, albums, songs"
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

            // Noctalia's segmented filter: one centred pill per scope, the
            // count inside it. The active scope is the only accented one.
            Item {
                Layout.fillWidth: true
                implicitHeight: root.panelRefHeight * 0.052

                Row {
                    anchors.centerIn: parent
										// Defines margin betwheen Chips of Scope Row
                     spacing: root.panelRefHeight * 0.025

                    Repeater {
                        model: root.scopeTitles

                        delegate: Item {
                            id: scopeChip
                            required property int index
                            required property string modelData

                            readonly property bool selected: root.scope === index
                            readonly property int count: root.scopeCount(index)
                            readonly property real labelSize: root.panelRefHeight * 0.0264
                            readonly property real labelScale: scopeChip.selected
                                ? 1.0 : (0.022 / 0.0264)
                            readonly property string labelText:
                                scopeChip.modelData + " " + scopeChip.count

                            FontMetrics {
                                id: selectedLabelMetrics
                                font.family: Theme.fonts.body
                                font.pixelSize: root.panelRefHeight * 0.0264
                                font.bold: true
                            }
                            readonly property real labelSlotWidth:
                                selectedLabelMetrics.advanceWidth(scopeChip.labelText)

                            implicitWidth: labelSlotWidth
                            implicitHeight: selectedLabelMetrics.height

                            // Fixed transparent slots: the text renders once at its selected
                            // size and scales inside the slot without reflowing the row.
                            Rectangle {
                                id: labelSlot
                                width: scopeChip.labelSlotWidth
                                height: parent.height
                                color: "transparent"

                                FadeText {
                                    anchors.centerIn: parent
                                    text: scopeChip.labelText
                                    color: scopeChip.selected
                                        ? Palette.primary : Palette.onSurfaceVariant
                                    opacity: scopeChip.selected ? 1.0 : 0.60
                                    Behavior on opacity {
                                        NumberAnimation {
                                            duration: Theme.motion.quick
                                            easing.type: Easing.Bezier
                                            easing.bezierCurve: Theme.ease.standard
                                        }
                                    }
                                    font.pixelSize: scopeChip.labelSize
                                    scalable: true
                                    scale: scopeChip.labelScale
                                    transformOrigin: Item.Center
                                    Behavior on scale {
                                        NumberAnimation {
                                            duration: Theme.motion.standard
                                            easing.type: Easing.Bezier
                                            easing.bezierCurve: Theme.ease.decelerate
                                        }
                                    }
                                    // font.bold: scopeChip.selected
                                    animated: true
                                }
                            }


                            HoverHandler {
                                id: chipHover
                                cursorShape: Qt.PointingHandCursor
                            }
                            TapHandler {
                                onTapped: root.setScope(scopeChip.index)
                            }
                        }
                    }
                }
            }

            // All scopes empty: one message in the panel itself replaces the
            // list (and the panel shrinks to fit it).
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

            // One unified result list. The scope strip is the only section
            // marker; the list itself is the shared result row anatomy.
            SearchColumn {
                id: resultsColumn
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: !root.nothingFound
                title: root.scopeTitles[root.scope]
                showHeader: false
                roundedArt: root.scope === 2
                model: root.scopeModel(root.scope)
                active: true
                loading: root.loading
                hasQuery: field.text.length > 0
                headerRefHeight: root.panelRefHeight
                quickActionKind: root.scope === 0 ? "song" : (root.scope === 1 ? "album" : "artist")
                selectionField: root.scope === 0 ? "songId" : (root.scope === 1 ? "albumId" : "artistId")

                onQuickAction: (target, actions) => root.quickAction(target, actions)

                onPicked: index => {
                    resultsColumn.cursor = index;
                    root.activate();
                }

                Behavior on opacity {
                    NumberAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
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
                text: root.typing
                    ? "Type to filter  ·  Ctrl-H/L or Tab scope  ·  ↑/↓ rows  ·  Enter open/play  ·  Esc normal mode"
                    : "Shift-H/L or Tab scope  ·  j/k rows  ·  i type  ·  Enter open/play  ·  Esc close"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.panelRefHeight)
                horizontalAlignment: Text.AlignHCenter
                // Vim's `wrap`: when the keybind tips do not fit the panel
                // width, continue them on the next line instead of clipping.
                wrapMode: Text.WordWrap
            }
        }
    }
}
