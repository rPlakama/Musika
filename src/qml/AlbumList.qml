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
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Central pane: the indexed album list, fed asynchronously from library.db.
// Clicking a row opens the album page.
Rectangle {
    id: root

    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)

    property string artistFilter: ""

    color: Palette.surface

    ListModel {
        id: albums
    }

    function reload() {
        Backend.requestAlbums(root.artistFilter);
    }

    function apply(payload) {
        albums.clear();
        var list = JSON.parse(payload);
        for (var i = 0; i < list.length; i++) {
            var album = list[i];
            albums.append({
                albumId: album.id,
                title: album.title,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count),
                cover: album.cover ? album.cover : ""
            });
        }
    }

    Component.onCompleted: reload()

    onArtistFilterChanged: reload()

    function activateCurrent() {
        var album = albums.get(list.currentIndex);
        if (album)
            root.albumActivated(album.albumId, album.title, album.artist, album.year, album.tracks, album.cover);
    }

    function handleAction(action) {
        if (albums.count === 0)
            return;
        if (action === "move_up" || action === "scroll_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down" || action === "scroll_down")
            list.currentIndex = Math.min(albums.count - 1, list.currentIndex + 1);
        else if (action === "enter")
            activateCurrent();
        list.positionViewAtIndex(list.currentIndex, ListView.Contain);
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "albums")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }

    FadeText {
        anchors.centerIn: parent
        visible: albums.count === 0
        text: "Nothing indexed yet — run a sync"
        color: Palette.onSurfaceVariant
        font.pixelSize: Theme.fontSize("body", root.height)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Section header.
        Item {
            Layout.fillWidth: true
            implicitHeight: root.height * 0.1

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: root.width * 0.02
                text: "Albums"
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: root.width * 0.02
                text: albums.count + " indexed"
                color: Palette.onSurfaceVariant
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
                required property string cover

                readonly property real viewWidth: ListView.view ? ListView.view.width : 0

                width: viewWidth
                height: ListView.view ? ListView.view.height * 0.105 : 0
                color: ListView.isCurrentItem
                    ? Palette.secondaryContainer
                    : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                Rectangle {
                    id: cover
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: row.viewWidth * 0.02
                    width: row.height * 0.78
                    height: row.height * 0.78
                    radius: Theme.radius
                    color: rowHover.hovered ? Palette.primaryContainer : Palette.surfaceContainerHigh
                    clip: true

                    CoverImage {
                        anchors.fill: parent
                        source: row.cover
                    }
                }

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: row.viewWidth * 0.02 + row.height * 0.78 + row.viewWidth * 0.02
                    spacing: row.height * 0.05

                    FadeText {
                        text: row.title
                        color: Palette.onSurface
                        font.pixelSize: row.height * 0.24
                        animated: false
                    }
                    FadeText {
                        text: row.artist + (row.year.length > 0 ? "  ·  " + row.year : "")
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.19
                        animated: false
                    }
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: row.viewWidth * 0.02
                    text: row.tracks + " tracks"
                    color: Palette.onSurfaceVariant
                    font.pixelSize: row.height * 0.19
                    animated: false
                }

                HoverHandler {
                    id: rowHover
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        list.currentIndex = row.index;
                        root.albumActivated(row.albumId, row.title, row.artist, row.year, row.tracks, row.cover);
                    }
                }
            }
        }
    }
}
