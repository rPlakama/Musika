import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Central pane: the indexed album list, fed from library.db. Clicking a row
// queues the album and starts playback.
Rectangle {
    id: root

    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string coverArt)

    color: Theme.surface

    ListModel {
        id: albums
    }

    function reload() {
        albums.clear();
        var list = JSON.parse(Backend.loadAlbums());
        for (var i = 0; i < list.length; i++) {
            var album = list[i];
            albums.append({
                albumId: album.id,
                title: album.title,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count),
                coverArt: album.cover_art ? album.cover_art : ""
            });
        }
    }

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onLibrary_changed() {
            root.reload();
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Section header.
        Item {
            Layout.fillWidth: true
            implicitHeight: root.height * 0.1

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: root.width * 0.02
                text: "Albums"
                color: Theme.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }

            Text {
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: root.width * 0.02
                text: albums.count + " indexed"
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.021
            }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: albums
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: row
                required property int index
                required property string albumId
                required property string title
                required property string artist
                required property string year
                required property string tracks
                required property string coverArt

                readonly property real viewWidth: ListView.view ? ListView.view.width : 0

                width: viewWidth
                height: ListView.view ? ListView.view.height * 0.105 : 0
                color: index % 2 === 0 ? Theme.surface : Theme.surfaceContainerLowest

                Rectangle {
                    id: cover
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: row.viewWidth * 0.02
                    width: row.height * 0.78
                    height: row.height * 0.78
                    radius: Theme.radius
                    color: rowHover.hovered ? Theme.primaryContainer : Theme.surfaceContainerHigh
                    clip: true

                    Image {
                        anchors.fill: parent
                        source: row.coverArt.length > 0 ? Backend.coverUrl(row.coverArt) : ""
                        fillMode: Image.PreserveAspectCrop
                        asynchronous: true
                        cache: false
                        visible: status === Image.Ready
                    }
                }

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: row.viewWidth * 0.02 + row.height * 0.78 + row.viewWidth * 0.02
                    spacing: row.height * 0.05

                    Text {
                        text: row.title
                        color: Theme.onSurface
                        font.pixelSize: row.height * 0.24
                    }
                    Text {
                        text: row.artist + (row.year.length > 0 ? "  ·  " + row.year : "")
                        color: Theme.onSurfaceVariant
                        font.pixelSize: row.height * 0.19
                    }
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: row.viewWidth * 0.02
                    text: row.tracks + " tracks"
                    color: Theme.onSurfaceVariant
                    font.pixelSize: row.height * 0.19
                }

                HoverHandler {
                    id: rowHover
                }

                TapHandler {
                    onTapped: root.albumActivated(row.albumId, row.title, row.artist, row.year, row.tracks, row.coverArt)
                }
            }
        }
    }
}
