import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Album entry: cover on top, then the album description, then its songs.
//
// Set `albumId` to load the real track list from library.db; clicking a song
// queues the album at that track. The descriptive fields are still placeholders
// until album metadata is passed in.
Rectangle {
    id: root

    signal back()

    property string albumId: ""
    property string albumCoverArt: ""
    property string albumTitle: "In Rainbows"
    property string albumArtist: "Radiohead"
    property string albumMeta: "2007   ·   Art rock, Alternative   ·   10 songs"
    property string albumNotes: "In Rainbows was released in 2007 as a pay-what-you-want download. Recorded in Oxfordshire and Wales, it moves through warmer, more rhythm-driven arrangements than its predecessor."

    color: Theme.surface

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        return Math.floor(total / 60) + ":" + ("0" + (total % 60)).slice(-2);
    }

    ListModel {
        id: songs
    }

    function reload() {
        songs.clear();
        if (root.albumId === "")
            return;
        var list = JSON.parse(Backend.loadAlbumSongs(root.albumId));
        for (var i = 0; i < list.length; i++) {
            var song = list[i];
            songs.append({
                track: song.track ? String(song.track) : String(i + 1),
                title: song.title,
                duration: root.formatTime(song.duration)
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
        anchors.margins: root.width * 0.02
        spacing: root.height * 0.02

        // Toolbar: back + play album.
        RowLayout {
            Layout.fillWidth: true
            spacing: root.width * 0.02

            Rectangle {
                implicitWidth: backLabel.implicitWidth + root.width * 0.03
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: backHover.hovered ? Theme.surfaceContainerHighest : Theme.surfaceContainerHigh

                Text {
                    id: backLabel
                    anchors.centerIn: parent
                    text: "‹  Back"
                    color: Theme.onSurface
                    font.pixelSize: root.height * 0.022
                }

                HoverHandler {
                    id: backHover
                }
                TapHandler {
                    onTapped: root.back()
                }
            }

            Item {
                Layout.fillWidth: true
            }

            Rectangle {
                implicitWidth: playLabel.implicitWidth + root.width * 0.03
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: playHover.hovered ? Theme.primary : Theme.primaryContainer

                Text {
                    id: playLabel
                    anchors.centerIn: parent
                    text: "▶  Play album"
                    color: Theme.onPrimaryContainer
                    font.pixelSize: root.height * 0.022
                }

                HoverHandler {
                    id: playHover
                }
                TapHandler {
                    onTapped: Backend.playAlbum(root.albumId, 0)
                }
            }
        }

        // Cover (top), kept square and centred.
        Item {
            id: coverBox
            Layout.fillWidth: true
            Layout.preferredHeight: root.height * 0.32

            Rectangle {
                anchors.centerIn: parent
                width: Math.min(coverBox.width, coverBox.height)
                height: width
                radius: Theme.radius
                color: Theme.primaryContainer
                clip: true

                Image {
                    id: coverImage
                    anchors.fill: parent
                    source: root.albumCoverArt.length > 0
                        ? Backend.coverUrl(root.albumCoverArt)
                        : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: false
                    visible: status === Image.Ready
                }

                Text {
                    anchors.centerIn: parent
                    visible: coverImage.status !== Image.Ready
                    text: "COVER"
                    color: Theme.onPrimaryContainer
                    font.pixelSize: coverBox.height * 0.12
                    font.letterSpacing: coverBox.height * 0.02
                }
            }
        }

        // Description.
        ColumnLayout {
            Layout.fillWidth: true
            spacing: root.height * 0.006

            Text {
                Layout.fillWidth: true
                text: root.albumTitle
                color: Theme.onSurface
                font.pixelSize: root.height * 0.036
                font.bold: true
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                text: root.albumArtist
                color: Theme.primary
                font.pixelSize: root.height * 0.024
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                text: root.albumMeta
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.02
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                Layout.topMargin: root.height * 0.008
                text: root.albumNotes
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.02
                lineHeight: 1.25
                wrapMode: Text.WordWrap
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Theme.outlineVariant
        }

        // Songs.
        Text {
            text: "Songs"
            color: Theme.primary
            font.pixelSize: root.height * 0.024
            font.bold: true
            font.letterSpacing: root.height * 0.004
        }

        ListView {
            id: songList
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: songs
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: songRow
                required property int index
                required property string track
                required property string title
                required property string duration

                width: songList.width
                height: root.height * 0.055
                color: songHover.hovered
                    ? Theme.surfaceContainerHigh
                    : (index % 2 === 0 ? Theme.surface : Theme.surfaceContainerLowest)

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    width: songRow.height * 0.6
                    text: songRow.track
                    color: Theme.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                    horizontalAlignment: Text.AlignRight
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: songRow.height * 0.8
                    anchors.right: songDuration.left
                    anchors.rightMargin: root.width * 0.02
                    text: songRow.title
                    color: Theme.onSurface
                    font.pixelSize: songRow.height * 0.36
                    elide: Text.ElideRight
                }
                Text {
                    id: songDuration
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: root.width * 0.02
                    text: songRow.duration
                    color: Theme.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                }

                HoverHandler {
                    id: songHover
                }

                TapHandler {
                    onTapped: Backend.playAlbum(root.albumId, songRow.index)
                }
            }
        }
    }
}
