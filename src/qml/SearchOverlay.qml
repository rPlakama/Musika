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
    property int activeColumn: 0 // 0 artists, 1 albums, 2 songs (roadmap priority)

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)
    signal artistActivated(string name)

    // Keep rendering while the close fade runs (the panel owns the opacity).
    visible: opened || panel.opacity > 0
    z: Theme.zOverlay

    function open() {
        openWith("");
    }

    /// Opens in typing mode with `text` already in the field. Paste (VI-21)
    /// uses this so the user sees the clipboard text and can correct it in
    /// place before choosing a result.
    function openWith(text) {
        opened = true;
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
        else if (action === "scroll_up")
            moveRow(-1);
        else if (action === "scroll_down")
            moveRow(1);
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
    }

    function columnFor(column) {
        if (column === 0)
            return artistsColumn;
        if (column === 1)
            return albumsColumn;
        return songsColumn;
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

    function activate() {
        var column = columnFor(activeColumn);
        var hit = column.model ? column.model.get(column.cursor) : null;
        if (!hit)
            return;
        if (activeColumn === 2)
            root.songActivated(hit.id);
        else if (activeColumn === 1)
            root.albumActivated(hit.id, hit.title, hit.artist, hit.year, hit.tracks, hit.cover);
        else
            root.artistActivated(hit.title);
        close();
    }

    function formatTime(seconds) {
        if (!seconds || seconds <= 0)
            return "";
        var minutes = Math.floor(seconds / 60);
        var rest = Math.floor(seconds % 60);
        return minutes + ":" + (rest < 10 ? "0" + rest : rest);
    }

    function apply(payload) {
        if (!root.opened || field.text.length === 0)
            return;

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
                subtitle: (song.artist ? song.artist : "") + (song.album ? "  ·  " + song.album : ""),
                meta: formatTime(song.duration),
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
            artistResults.append({
                id: artist.id,
                title: artist.name,
                subtitle: "",
                meta: artist.album_count + " albums",
                cover: artist.cover
            });
        }

        songsColumn.cursor = songResults.count > 0 ? 0 : -1;
        albumsColumn.cursor = albumResults.count > 0 ? 0 : -1;
        artistsColumn.cursor = artistResults.count > 0 ? 0 : -1;
        // Lookup priority picks the column the cursor starts in.
        activeColumn = artistResults.count > 0 ? 0 : (albumResults.count > 0 ? 1 : 2);

        loading = false;
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
            NumberAnimation { duration: Theme.motion.standard; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
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
        color: Palette.surfaceContainerHigh
        radius: Theme.radius
        border.color: Palette.primary
        border.width: Math.max(1, root.height * Theme.hairlineRatio)
        opacity: root.opened ? 1 : 0

        Behavior on opacity {
            NumberAnimation { duration: Theme.motion.standard; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.decelerate }
        }
        Behavior on y {
            NumberAnimation { duration: Theme.motion.standard; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.decelerate }
        }

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: panel.height * 0.035
            spacing: panel.height * 0.025

            RowLayout {
                Layout.fillWidth: true
                spacing: panel.width * 0.01

                FadeText {
                    text: "⌕"
                    color: Palette.onSurfaceVariant
                    font.pixelSize: panel.height * 0.05
                }

                TextField {
                    id: field
                    Layout.fillWidth: true
                    placeholderText: "Search anything"
                    placeholderTextColor: Palette.onSurfaceVariant
                    color: Palette.onSurface
                    font.pixelSize: panel.height * 0.035
                    background: Rectangle {
                        color: Palette.surfaceContainerHighest
                        radius: Theme.radius
                        border.color: field.activeFocus ? Palette.primary : Palette.outlineVariant
                        border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    }

                    onTextChanged: debounce.restart()

                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Tab) {
                            root.moveColumn(1);
                            event.accepted = true;
                        } else if (event.key === Qt.Key_Backtab) {
                            root.moveColumn(-1);
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
                    Keys.onEscapePressed: (event) => {
                        root.pressEscape();
                        event.accepted = true;
                    }
                }

                // Mode chip: INSERT while typing, NORMAL while navigating.
                Rectangle {
                    implicitWidth: modeChipLabel.implicitWidth + panel.height * 0.05
                    implicitHeight: panel.height * 0.06
                    radius: Theme.radius
                    color: Palette.surfaceContainerHighest
                    border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    border.color: root.typing ? Palette.tertiary : Palette.primary

                    FadeText {
                        id: modeChipLabel
                        anchors.centerIn: parent
                        text: root.typing ? "INSERT" : "NORMAL"
                        color: root.typing ? Palette.tertiary : Palette.primary
                        font.pixelSize: Theme.fontSize("label", panel.height)
                        font.bold: true
                        font.letterSpacing: panel.height * 0.006
                        animated: false
                    }
                }
            }

            // Column widths are RowLayout weights (28/32/40), not pixels.
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: panel.width * 0.012

                SearchColumn {
                    id: artistsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 28
                    Layout.fillHeight: true
                    title: "Artists"
                    roundedArt: true
                    model: artistResults
                    active: root.activeColumn === 0
                    loading: root.loading
                    hasQuery: field.text.length > 0

                    onHovered: (index) => {
                        root.activeColumn = 0;
                        artistsColumn.cursor = index;
                    }
                    onPicked: (index) => {
                        artistsColumn.cursor = index;
                        root.activeColumn = 0;
                        root.activate();
                    }
                }

                SearchColumn {
                    id: albumsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 32
                    Layout.fillHeight: true
                    title: "Albums"
                    model: albumResults
                    active: root.activeColumn === 1
                    loading: root.loading
                    hasQuery: field.text.length > 0

                    onHovered: (index) => {
                        root.activeColumn = 1;
                        albumsColumn.cursor = index;
                    }
                    onPicked: (index) => {
                        albumsColumn.cursor = index;
                        root.activeColumn = 1;
                        root.activate();
                    }
                }

                SearchColumn {
                    id: songsColumn
                    Layout.fillWidth: true
                    Layout.preferredWidth: 40
                    Layout.fillHeight: true
                    title: "Songs"
                    model: songResults
                    active: root.activeColumn === 2
                    loading: root.loading
                    hasQuery: field.text.length > 0

                    onHovered: (index) => {
                        root.activeColumn = 2;
                        songsColumn.cursor = index;
                    }
                    onPicked: (index) => {
                        songsColumn.cursor = index;
                        root.activeColumn = 2;
                        root.activate();
                    }
                }
            }

            // One cheat sheet, per mode (no duplicate Esc hint).
            FadeText {
                Layout.fillWidth: true
                text: root.typing
                    ? "Type to filter   ·   Tab/↑/↓ navigate   ·   Enter open/play   ·   Esc normal mode"
                    : "h/l columns   ·   j/k rows   ·   i type   ·   Enter open/play   ·   Esc close"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", panel.height)
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
