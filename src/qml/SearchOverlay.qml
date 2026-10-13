import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Floating global search (Shift Space F): one panel with Songs | Albums |
// Artists columns, filtered live from the library. It is a modal layer opened
// from any region; typing filters, Tab/h/l switch columns, Up/Down walk rows,
// Enter opens/plays, Esc closes and returns focus to the previous region.
Item {
    id: root

    property bool opened: false
    property bool loading: false
    property int activeColumn: 0 // 0 artists, 1 albums, 2 songs (roadmap priority)

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)
    signal artistActivated(string name)

    visible: opened
    z: 30

    function open() {
        opened = true;
        field.text = "";
        clearResults();
        activeColumn = 0;
        loading = false;
        field.forceActiveFocus();
    }

    function close() {
        opened = false;
        Backend.releaseFocus();
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
                meta: artist.album_count + " alb",
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

    // Dim backdrop; click outside to dismiss.
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: root.opened ? 0.5 : 0

        Behavior on opacity {
            NumberAnimation { duration: Theme.duration }
        }

        TapHandler {
            onTapped: root.close()
        }
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: root.width * 0.70
        height: root.height * 0.65
        color: Theme.surfaceContainerHigh
        radius: Theme.radius
        border.color: Theme.primary
        border.width: Math.max(1, root.height * Theme.hairlineRatio)
        opacity: root.opened ? 1 : 0
        scale: root.opened ? 1 : 0.98

        Behavior on opacity {
            NumberAnimation { duration: Theme.duration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.easeOut }
        }
        Behavior on scale {
            NumberAnimation { duration: Theme.duration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.easeOut }
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
                    color: Theme.onSurfaceVariant
                    font.pixelSize: panel.height * 0.05
                }

                TextField {
                    id: field
                    Layout.fillWidth: true
                    placeholderText: "Search songs, albums and artists…"
                    placeholderTextColor: Theme.onSurfaceVariant
                    color: Theme.onSurface
                    font.pixelSize: panel.height * 0.045
                    background: Rectangle {
                        color: Theme.surfaceContainerHighest
                        radius: Theme.radius
                        border.color: field.activeFocus ? Theme.primary : Theme.outlineVariant
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
                        }
                    }
                    Keys.onDownPressed: root.moveRow(1)
                    Keys.onUpPressed: root.moveRow(-1)
                    Keys.onReturnPressed: root.activate()
                    Keys.onEnterPressed: root.activate()
                    Keys.onEscapePressed: root.close()
                }

                FadeText {
                    text: "Esc close"
                    color: Theme.onSurfaceVariant
                    font.pixelSize: panel.height * 0.032
                }
            }

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

            FadeText {
                Layout.fillWidth: true
                text: "Enter open/play   ·   Tab column   ·   ↑/↓ row   ·   Esc close"
                color: Theme.onSurfaceVariant
                font.pixelSize: panel.height * 0.03
                horizontalAlignment: Text.AlignHCenter
            }
        }
    }
}
