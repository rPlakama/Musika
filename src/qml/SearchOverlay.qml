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
    // UI-49: last query that already got the denied cue, so one response (or
    // one Enter) lights it once and holding a zero query never strobes.
    property string _zeroQuery: ""

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)
    signal artistActivated(string artistId, string artistName, string artistCover)

    // Keep rendering while the close fade runs (the panel owns the opacity).
    visible: opened || panel.opacity > 0
    z: Theme.zOverlay

    // UI-49: one reusable denied-input cue (shake + bounce); the red glyph
    // flash below rides the same call.
    ShakeFx {
        id: denyFx
        target: field
    }

    SequentialAnimation {
        id: glyphFlash
        NumberAnimation {
            target: glyphFlashIcon
            property: "opacity"
            to: 1
            duration: Theme.motion.flash
        }
        NumberAnimation {
            target: glyphFlashIcon
            property: "opacity"
            to: 0
            duration: Theme.motion.quick
        }
    }

    function denied() {
        if (Theme.reduceMotion || field.text.length === 0 || denyFx.swing.running)
            return;
        denyFx.shake();
        glyphFlash.restart();
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
        loading = text.length > 0;
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

    function moveRow(delta) {
        var column = columnFor(activeColumn);
        if (!column.model || column.model.count === 0)
            return;
        var next = Math.max(0, Math.min(column.model.count - 1, column.cursor + delta));
        column.cursor = next;
    }

    function moveColumn(delta) {
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
        if (!root.opened || field.text.length === 0)
            return;

        // UI-41: results crossfade in on payload swaps instead of flashing.
        columnsRow.opacity = 0;
        songResults.clear();
        albumResults.clear();
        artistResults.clear();

        var data = JSON.parse(payload);
        var i;
        for (i = 0; i < data.songs.length; i++) {
            var song = data.songs[i];
            songResults.append({
                id: song.id,
                title: song.title,
                subtitle: (song.artist ? song.artist : "") + (song.album ? " · " + song.album : ""),
                meta: Theme.formatTime(song.duration),
                cover: song.cover
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
                tracks: String(album.song_count)
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
                cover: artist.cover
            });
        }

        songsColumn.cursor = songResults.count > 0 ? 0 : -1;
        albumsColumn.cursor = albumResults.count > 0 ? 0 : -1;
        artistsColumn.cursor = artistResults.count > 0 ? 0 : -1;
        // Lookup priority picks the column the cursor starts in.
        activeColumn = songResults.count > 0 ? 0 : (albumResults.count > 0 ? 1 : 2);

        loading = false;
        Qt.callLater(function () {
            columnsRow.opacity = 1;
        });

        // UI-49: the response landing on zero hits is denied input — once per
        // distinct query, never again while the same zero query is held.
        var total = songResults.count + albumResults.count + artistResults.count;
        if (total === 0) {
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
            if (field.text.length > 0) {
                root.loading = true;
                Backend.requestSearch(field.text);
            } else {
                root.clearResults();
                root.loading = false;
            }
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

    Rectangle {
        id: panel
        width: root.width * 0.70
        height: root.height * 0.65
        // Rise into place (preferred over scale, motion §3.1).
        x: (root.width - width) / 2
        y: (root.height - height) / 2 + (root.opened ? 0 : height * 0.04)
        // Same darkest tone as the main pane (Palette.surface); the field and
        // chips keep the higher container tones for contrast.
        color: Palette.surface
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

            RowLayout {
                Layout.fillWidth: true
                spacing: panel.width * 0.01

                TextField {
                    id: field
                    Layout.fillWidth: true
                    leftPadding: panel.height * 0.092
                    placeholderText: "Search anything"
                    placeholderTextColor: Palette.onSurfaceVariant
                    color: Palette.onSurface
                    font.family: Theme.fonts.body
                    font.pixelSize: panel.height * 0.035
                    // UI-49: the denied shake is a transform, never a layout move.
                    transform: Translate {
                        x: denyFx.offset
                    }
                    background: Rectangle {
                        color: Palette.surfaceContainerHighest
                        radius: Theme.radius
                        border.color: field.activeFocus ? Palette.primary : Palette.outlineVariant
                        border.width: Math.max(1, root.height * Theme.hairlineRatio)

                        // The glyph lives inside the field's own left padding,
                        // inset from the border with a gap before the text, so
                        // the input edge still lines up with the columns below.
                        // Fixed white like the other SVG glyphs (Theme.iconStrong).
                        SymbolIcon {
                            id: searchGlyph
                            anchors.left: parent.left
                            anchors.leftMargin: panel.height * 0.03
                            anchors.verticalCenter: parent.verticalCenter
                            width: panel.height * 0.04
                            height: width
                            source: "qrc:/app/icons/search-right-1505-svgrepo-com.svg"
                            symbolColor: Theme.iconStrong
                        }

                        // UI-49: the glyph asset is fixed white, so the denied
                        // flash is a red-tinted copy fading over it.
                        SymbolIcon {
                            id: glyphFlashIcon
                            anchors.fill: searchGlyph
                            source: searchGlyph.source
                            symbolColor: Palette.error
                            opacity: 0
                        }
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

                // Mode chip: INSERT while typing, NORMAL while navigating.
                // Same geometry and accent as the superior-bar NORMAL chip.
                Rectangle {
                    implicitWidth: modeChipLabel.implicitWidth + panel.height * 0.05
                    implicitHeight: panel.height * 0.06
                    radius: Theme.radius
                    color: Palette.surfaceContainerHighest
                    border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    border.color: Palette.primary

                    FadeText {
                        id: modeChipLabel
                        anchors.centerIn: parent
                        text: root.typing ? "I" : "N"
                        color: Palette.primary
                        font.pixelSize: Theme.fontSize("label", panel.height)
                        animated: false
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
                    title: "Songs"
                    model: songResults
                    active: root.activeColumn === 0
                    loading: root.loading
                    hasQuery: field.text.length > 0

                    onPicked: index => {
                        songsColumn.cursor = index;
                        root.activeColumn = 0;
                        root.activate();
                    }
                }

                // Structural rule: keeps each column's header/count contained
                // instead of bleeding into its neighbour.
                Rectangle {
                    Layout.fillHeight: true
                    Layout.preferredWidth: Math.max(1, panel.height * Theme.hairlineRatio)
                    color: Palette.outlineVariant
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
