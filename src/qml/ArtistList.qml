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

// All indexed artists. Enter (or click) opens the artist's albums.
Rectangle {
    id: root

    signal artistActivated(string artist)

    color: Palette.surface

    ListModel {
        id: artists
    }

    function reload() {
        Backend.requestArtists();
    }

    function apply(payload) {
        artists.clear();
        var list = JSON.parse(payload);
        for (var i = 0; i < list.length; i++) {
            var artist = list[i];
            artists.append({
                artistName: artist.name,
                albumCount: String(artist.album_count),
                cover: artist.cover ? artist.cover : ""
            });
        }
    }

    function activateCurrent() {
        var artist = artists.get(list.currentIndex);
        if (artist)
            root.artistActivated(artist.artistName);
    }

    function handleAction(action) {
        if (artists.count === 0)
            return;
        if (action === "move_up" || action === "scroll_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down" || action === "scroll_down")
            list.currentIndex = Math.min(artists.count - 1, list.currentIndex + 1);
        else if (action === "enter")
            activateCurrent();
        list.positionViewAtIndex(list.currentIndex, ListView.Contain);
    }

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artists")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }

    FadeText {
        anchors.centerIn: parent
        visible: artists.count === 0
        text: "Nothing indexed yet — run a sync"
        color: Palette.onSurfaceVariant
        font.pixelSize: Theme.fontSize("body", root.height)
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Item {
            Layout.fillWidth: true
            implicitHeight: root.height * 0.1

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: root.width * 0.02
                text: "Artists"
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }
            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: root.width * 0.02
                text: artists.count + " indexed"
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.021
            }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: artists
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: row
                required property int index
                required property string artistName
                required property string albumCount
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
                    radius: width * 0.5
                    color: Palette.surfaceContainerHigh
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
                        text: row.artistName
                        color: Palette.onSurface
                        font.pixelSize: row.height * 0.26
                        animated: false
                    }
                    FadeText {
                        text: row.albumCount + " albums"
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.2
                        animated: false
                    }
                }

                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        list.currentIndex = row.index;
                        root.artistActivated(row.artistName);
                    }
                }
            }
        }
    }
}
