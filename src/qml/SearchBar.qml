import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Global search bar (artists > albums > songs) with a results dropdown.
Rectangle {
    id: root

    signal songActivated(string id)
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)
    signal artistActivated(string name)

    implicitHeight: parent ? parent.height * 0.085 : 0
    color: Theme.surface
    z: 2

    function focusSearch() {
        field.forceActiveFocus();
        field.selectAll();
    }

    function close() {
        results.clear();
        field.text = "";
        field.focus = false;
        Backend.releaseFocus();
    }

    function apply(payload) {
        results.clear();
        var data = JSON.parse(payload);
        var i;
        for (i = 0; i < data.artists.length; i++) {
            var artist = data.artists[i];
            results.append({
                kind: "artist",
                entryId: artist.id,
                title: artist.name,
                subtitle: artist.album_count + " albums",
                cover: artist.cover
            });
        }
        for (i = 0; i < data.albums.length; i++) {
            var album = data.albums[i];
            results.append({
                kind: "album",
                entryId: album.id,
                title: album.title,
                subtitle: (album.artist ? album.artist : "") + (album.year ? "  ·  " + album.year : ""),
                cover: album.cover,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count)
            });
        }
        for (i = 0; i < data.songs.length; i++) {
            var song = data.songs[i];
            results.append({
                kind: "song",
                entryId: song.id,
                title: song.title,
                subtitle: (song.artist ? song.artist : "") + (song.album ? "  ·  " + song.album : ""),
                cover: song.cover
            });
        }
        list.currentIndex = results.count > 0 ? 0 : -1;
    }

    function activate() {
        var hit = results.get(list.currentIndex);
        if (!hit)
            return;
        if (hit.kind === "song")
            root.songActivated(hit.entryId);
        else if (hit.kind === "album")
            root.albumActivated(hit.entryId, hit.title, hit.artist, hit.year, hit.tracks, hit.cover);
        else
            root.artistActivated(hit.title);
        close();
    }

    ListModel {
        id: results
    }

    Timer {
        id: debounce
        interval: 150
        onTriggered: {
            if (field.text.length > 0)
                Backend.requestSearch(field.text);
            else
                results.clear();
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "search" && field.activeFocus)
                root.apply(payload);
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.02
        anchors.rightMargin: root.width * 0.02
        spacing: root.height * 0.15

        FadeText {
            text: "⌕"
            color: Theme.onSurfaceVariant
            font.pixelSize: root.height * 0.38
        }

        TextField {
            id: field
            Layout.fillWidth: true
            placeholderText: "Search artists, albums and songs…"
            color: Theme.onSurface
            placeholderTextColor: Theme.onSurfaceVariant
            font.pixelSize: root.height * 0.27
            background: Rectangle {
                color: Theme.surfaceContainerHigh
                radius: Theme.radius
                border.color: field.activeFocus ? Theme.primary : Theme.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }

            onTextChanged: debounce.restart()

            Keys.onDownPressed: list.currentIndex = Math.min(results.count - 1, list.currentIndex + 1)
            Keys.onUpPressed: list.currentIndex = Math.max(0, list.currentIndex - 1)
            Keys.onReturnPressed: root.activate()
            Keys.onEnterPressed: root.activate()
            Keys.onEscapePressed: root.close()
        }

        // Hint chip for the floating palette.
        Rectangle {
            implicitWidth: hint.implicitWidth + root.height * 0.3
            implicitHeight: root.height * 0.5
            radius: Theme.radius
            color: Theme.surfaceContainerHigh

            FadeText {
                id: hint
                anchors.centerIn: parent
                text: "Shift Space F"
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.22
            }
        }
    }

    // Results dropdown.
    Rectangle {
        id: dropdown
        anchors.top: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        color: Theme.surfaceContainerHigh
        border.color: Theme.outlineVariant
        border.width: Math.max(1, root.height * Theme.hairlineRatio)
        visible: field.activeFocus && field.text.length > 0 && results.count > 0
        height: visible ? Math.min(results.count, 8) * (root.height * 0.82) : 0
        clip: true

        ListView {
            id: list
            anchors.fill: parent
            clip: true
            model: results
            currentIndex: -1
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: row
                required property int index
                required property string kind
                required property string entryId
                required property string title
                required property string subtitle
                required property string cover

                width: list.width
                height: root.height * 0.82
                color: index === list.currentIndex
                    ? Theme.secondaryContainer
                    : (index % 2 === 0 ? Theme.surfaceContainerHigh : Theme.surfaceContainer)

                Rectangle {
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: root.width * 0.01
                    width: row.height * 0.7
                    height: row.height * 0.7
                    radius: row.kind === "artist" ? width * 0.5 : Theme.radius
                    color: Theme.surfaceContainerHighest
                    clip: true

                    Image {
                        anchors.fill: parent
                        source: row.cover
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        cache: false
                        visible: status === Image.Ready
                    }
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: root.width * 0.01 + row.height * 0.7 + root.width * 0.01
                    anchors.right: kindLabel.left
                    anchors.rightMargin: root.width * 0.01
                    text: row.title + (row.subtitle.length > 0 ? "   " + row.subtitle : "")
                    color: index === list.currentIndex ? Theme.onSecondaryContainer : Theme.onSurface
                    font.pixelSize: row.height * 0.34
                    elide: Text.ElideRight
                }

                FadeText {
                    id: kindLabel
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: root.width * 0.01
                    text: row.kind
                    color: Theme.onSurfaceVariant
                    font.pixelSize: row.height * 0.26
                }

                TapHandler {
                    onTapped: {
                        list.currentIndex = row.index;
                        root.activate();
                    }
                }
            }
        }
    }
}
