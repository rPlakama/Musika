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

// Floating global search (Shift Space F): one modal panel with Artists |
// Albums | Songs columns (RowLayout weights 28/32/40), filtered live from the
// library. Typing mode: keys type, Tab/arrows navigate, Esc -> normal mode.
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

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)
    signal artistActivated(string artistId, string artistName, string artistCover)
    /// ARC-I: Alt-hold quick actions armed on the active column's cursor row;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    // Keep rendering while the close fade runs (the panel owns the opacity).
    visible: opened || panel.opacity > 0
    z: Theme.zOverlay

    // UI-49: one reusable denied-input cue (shake + bounce); the red mode-chip
    // flash below rides the same call.
    ShakeFx {
        id: denyFx
        target: field
    }

    SequentialAnimation {
        id: modeFlash
        NumberAnimation {
            target: modeFlashLabel
            property: "opacity"
            to: 1
            duration: Theme.motion.flash
        }
        NumberAnimation {
            target: modeFlashLabel
            property: "opacity"
            to: 0
            duration: Theme.motion.quick
        }
    }

    function denied() {
        if (Theme.reduceMotion || field.text.length === 0 || denyFx.swing.running)
            return;
        denyFx.shake();
        modeFlash.restart();
    }

    function open() {
        openWith("");
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
            Backend.requestSearch("");
        typing = true;
        field.forceActiveFocus();
        field.cursorPosition = field.text.length;
    }

    function close() {
        opened = false;
        typing = false;
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
        typing = true;
        field.forceActiveFocus();
        field.selectAll();
    }

    // Readline/vi-insert behaviour: Ctrl-W removes the word before the caret
    // (trailing spaces first, then the word itself).
    function deleteWordBack() {
        var text = field.text;
        var cursor = field.cursorPosition;
        var start = cursor;
        while (start > 0 && /\s/.test(text.charAt(start - 1)))
            start--;
        while (start > 0 && !/\s/.test(text.charAt(start - 1)))
            start--;
        if (start === cursor)
            return;
        field.text = text.slice(0, start) + text.slice(cursor);
        field.cursorPosition = start;
    }

    // Search normal mode: hjkl navigates, Enter opens/plays, i types again.
    function handleAction(action) {
        if (action === "back")
            close();
        else if (action === "insert_mode")
            enterTyping();
        else if (action === "move_left")
            moveColumn(-1);
        else if (action === "move_right")
            moveColumn(1);
        else if (action === "move_up")
            moveRow(-1);
        else if (action === "move_down")
            moveRow(1);
        else if (action === "top" || action === "bottom")
            moveEdge(action === "bottom");
        else if (action === "enter")
            activate();
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
        if (activeColumn === 0)
            root.songActivated(hit.id);
        else if (activeColumn === 1)
            root.albumActivated(hit.id, hit.title, hit.artist, hit.year, hit.tracks, hit.cover);
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
                albumId: song.album_id ? song.album_id : ""
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
                albumId: album.id
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
                artistId: artist.id
            });
        }

        songsColumn.cursor = songResults.count > 0 ? 0 : -1;
        albumsColumn.cursor = albumResults.count > 0 ? 0 : -1;
        artistsColumn.cursor = artistResults.count > 0 ? 0 : -1;
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
                Backend.requestSearch("");
                return;
            }
            Backend.requestSearch(field.text);
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
        height: root.height * 0.78
        // Rise into place (preferred over scale, motion §3.1).
        x: (root.width - width) / 2
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
                easing.bezierCurve: Theme.ease.decelerate
            }
        }
        Behavior on y {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.decelerate
            }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: panel.height * 0.035
            spacing: panel.height * 0.025

            // Mode chip + search bar in one parent rectangle, so the spacing
            // is explicit: 10% of the row is the chip (beside the bar, never
            // inside it), the rest is the field, with a margin between them
            // and the chip inset vertically.
            Rectangle {
                id: searchRow
                Layout.fillWidth: true
                implicitHeight: panel.height * 0.06
                color: "transparent"

                Rectangle {
                    id: modeChip
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width * 0.10
                    height: parent.height * 0.82
                    radius: Theme.radius
                    color: Palette.surfaceContainerHighest
                    border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    border.color: Palette.primary

                    // INSERT while typing, NORMAL while navigating.
                    FadeText {
                        id: modeChipLabel
                        anchors.centerIn: parent
                        text: root.typing ? "I" : "N"
                        color: Palette.primary
                        font.pixelSize: Theme.fontSize("label", panel.height)
                        animated: false
                    }

                    // UI-49: the denied cue's red copy of the label, fading in
                    // over the normal one.
                    FadeText {
                        id: modeFlashLabel
                        anchors.centerIn: parent
                        text: modeChipLabel.text
                        color: Palette.error
                        font.pixelSize: Theme.fontSize("label", panel.height)
                        opacity: 0
                        animated: false
                    }
                }

                TextField {
                    id: field
                    anchors.left: modeChip.right
                    anchors.leftMargin: parent.width * 0.012
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    placeholderText: "Search anything"
                    placeholderTextColor: Palette.onSurfaceVariant
                    color: Palette.onSurface
                    font.family: Theme.fonts.body
                    font.pixelSize: panel.height * 0.030
                    // UI-49: the denied shake is a transform, never a layout move.
                    transform: Translate {
                        x: denyFx.offset
                    }
                    background: Rectangle {
                        color: Palette.surfaceContainerHighest
                        radius: Theme.radius
                        border.color: field.activeFocus ? Palette.primary : Palette.outlineVariant
                        border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    }

                    onTextChanged: debounce.restart()

                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_Tab) {
                            root.moveColumn(1);
                            event.accepted = true;
                        } else if (event.key === Qt.Key_Backtab) {
                            root.moveColumn(-1);
                            event.accepted = true;
                        } else if (event.key === Qt.Key_W && (event.modifiers & Qt.ControlModifier)) {
                            root.deleteWordBack();
                            event.accepted = true;
                        } else if (event.key === Qt.Key_N && (event.modifiers & Qt.ControlModifier)) {
                            root.moveRow(1);
                            event.accepted = true;
                        } else if (event.key === Qt.Key_P && (event.modifiers & Qt.ControlModifier)) {
                            root.moveRow(-1);
                            event.accepted = true;
                        }
                    }
                    Keys.onDownPressed: root.moveRow(1)
                    Keys.onUpPressed: root.moveRow(-1)
                    Keys.onReturnPressed: root.activate()
                    Keys.onEnterPressed: root.activate()
                    Keys.onEscapePressed: event => {
                        root.pressEscape();
                        event.accepted = true;
                    }
                }
            }

            // Column widths are RowLayout weights (38/40/22), not pixels:
            // Albums gets the room its titles need, Artists is narrow since it
            // only carries a name plus its count.
            RowLayout {
                id: columnsRow
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: panel.width * 0.012

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
                    quickActionKind: "song"

                    onQuickAction: (target, actions) => root.quickAction(target, actions)

                    onPicked: index => {
                        songsColumn.cursor = index;
                        root.activeColumn = 0;
                        root.activate();
                    }
                }

                // Structural rule: keeps each column's header/count contained
                // instead of bleeding into its neighbour. UI-47: above the
                // next column's covers, which otherwise paint over it.
                Rectangle {
                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(1, panel.height * Theme.hairlineRatio)
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
                    quickActionKind: "album"

                    onQuickAction: (target, actions) => root.quickAction(target, actions)

                    onPicked: index => {
                        albumsColumn.cursor = index;
                        root.activeColumn = 1;
                        root.activate();
                    }
                }

                Rectangle {
                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(1, panel.height * Theme.hairlineRatio)
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
                    quickActionKind: "artist"

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
                Layout.fillWidth: true
                Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
                color: Palette.outlineVariant
            }

            // One cheat sheet, per mode (no duplicate Esc hint).
            FadeText {
                Layout.fillWidth: true
                text: root.typing ? "Type to filter  ·  Tab/↑/↓ navigate  ·  Enter open/play  ·  Esc normal mode" : "h/l columns  ·  j/k rows  ·  i type  ·  Enter open/play  ·  Esc close"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", panel.height)
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
