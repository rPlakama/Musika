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
    property bool onToolbar: false
    property string albumCover: ""
    property string albumTitle: ""
    property string albumArtist: ""
    property string albumMeta: ""
    property string albumNotes: ""

    color: Palette.surface

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        return Math.floor(total / 60) + ":" + ("0" + (total % 60)).slice(-2);
    }

    ListModel {
        id: songs
    }

    function reload() {
        if (root.albumId === "")
            return;
        Backend.requestAlbumSongs(root.albumId);
        Backend.requestAlbumInfo(root.albumId);
    }

    function applyInfo(payload) {
        var info = JSON.parse(payload);
        root.albumNotes = info.notes ? info.notes : "";
    }

    function apply(payload) {
        songs.clear();
        var list = JSON.parse(payload);
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

    function handleAction(action) {
        var up = action === "move_up" || action === "scroll_up";
        var down = action === "move_down" || action === "scroll_down";

        if (root.onToolbar) {
            // Cursor is on the toolbar (Back). Down returns to the songs.
            if (down && songs.count > 0) {
                root.onToolbar = false;
                songList.currentIndex = 0;
                songList.positionViewAtIndex(0, ListView.Contain);
            } else if (action === "enter") {
                root.back();
            }
            return;
        }

        if (up) {
            if (songList.currentIndex > 0)
                songList.currentIndex -= 1;
            else
                root.onToolbar = true;
        } else if (down) {
            if (songs.count > 0)
                songList.currentIndex = Math.min(songs.count - 1, songList.currentIndex + 1);
        } else if (action === "enter") {
            Backend.playAlbum(root.albumId, songList.currentIndex);
        }
        songList.positionViewAtIndex(songList.currentIndex, ListView.Contain);
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "album_songs")
                root.apply(payload);
            else if (channel === "album_info")
                root.applyInfo(payload);
        }
        function onLibrary_changed() {
            root.reload();
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

            Rectangle {
                id: backButton
                implicitWidth: backLabel.implicitWidth + root.width * 0.04
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: (root.onToolbar || backHover.hovered)
                    ? Palette.secondaryContainer
                    : Palette.surfaceContainerHigh
                border.color: root.onToolbar ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.duration
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.easeOut
                    }
                }

                FadeText {
                    id: backLabel
                    anchors.centerIn: parent
                    text: "<- ESC"
                    color: Palette.onSurface
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
                color: Palette.primaryContainer
                clip: true

                Image {
                    id: coverImage
                    anchors.fill: parent
                    source: root.albumCover
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: false
                    visible: status === Image.Ready
                }

                FadeText {
                    anchors.centerIn: parent
                    visible: coverImage.status !== Image.Ready
                    text: "COVER"
                    color: Palette.onPrimaryContainer
                    font.pixelSize: coverBox.height * 0.12
                    font.letterSpacing: coverBox.height * 0.02
                }
            }
        }

        // Description.
        ColumnLayout {
            Layout.fillWidth: true
            spacing: root.height * 0.006

            FadeText {
                Layout.fillWidth: true
                text: root.albumTitle
                color: Palette.onSurface
                font.pixelSize: root.height * 0.036
                font.bold: true
                elide: Text.ElideRight
            }
            FadeText {
                Layout.fillWidth: true
                text: root.albumArtist
                color: Palette.primary
                font.pixelSize: root.height * 0.024
                elide: Text.ElideRight
            }
            FadeText {
                Layout.fillWidth: true
                text: root.albumMeta
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.02
                elide: Text.ElideRight
            }
            FadeText {
                Layout.fillWidth: true
                Layout.topMargin: root.height * 0.008
                visible: root.albumNotes.length > 0
                text: root.albumNotes
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.02
                lineHeight: 1.25
                wrapMode: Text.WordWrap
                maximumLineCount: 3
                elide: Text.ElideRight
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Palette.outlineVariant
        }

        // Songs.
        FadeText {
            text: "Songs"
            color: Palette.primary
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
                color: (ListView.isCurrentItem && !root.onToolbar)
                    ? Palette.secondaryContainer
                    : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.duration
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.easeOut
                    }
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    width: songRow.height * 0.6
                    text: songRow.track
                    color: Palette.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                    horizontalAlignment: Text.AlignRight
                    animated: false
                }
                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: songRow.height * 0.8
                    anchors.right: songDuration.left
                    anchors.rightMargin: root.width * 0.02
                    text: songRow.title
                    color: Palette.onSurface
                    font.pixelSize: songRow.height * 0.36
                    elide: Text.ElideRight
                    animated: false
                }
                FadeText {
                    id: songDuration
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: root.width * 0.02
                    text: songRow.duration
                    color: Palette.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                    animated: false
                }

                TapHandler {
                    onTapped: {
                        songList.currentIndex = songRow.index;
                        Backend.playAlbum(root.albumId, songRow.index);
                    }
                }
            }
        }
    }
}
