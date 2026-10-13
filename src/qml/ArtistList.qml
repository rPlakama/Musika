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

// All indexed artists. Enter (or click) opens the artist entry.
Rectangle {
    id: root

    signal artistActivated(string artistId, string artistName, string artistCover)

    color: Palette.surface

    ListModel {
        id: artists
    }

    function reload() {
        Backend.requestArtists();
    }

    function apply(payload) {
        artists.clear();
        var rows = JSON.parse(payload);
        for (var i = 0; i < rows.length; i++) {
            var artist = rows[i];
            artists.append({
                artistId: artist.id,
                artistName: artist.name,
                albumCount: String(artist.album_count),
                cover: artist.cover ? artist.cover : ""
            });
        }
        // UI-10: a cursor row exists from the first paint.
        if (artists.count > 0 && (list.currentIndex < 0 || list.currentIndex >= artists.count))
            list.currentIndex = 0;
    }

    function activateCurrent() {
        var artist = artists.get(list.currentIndex);
        if (artist)
            root.artistActivated(artist.artistId, artist.artistName, artist.cover);
    }

    function handleAction(action) {
        if (artists.count === 0)
            return;
        if (action === "move_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down")
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
                required property string artistId
                required property string artistName
                required property string albumCount
                required property string cover

                readonly property real viewWidth: ListView.view ? ListView.view.width : 0

                width: viewWidth
                height: ListView.view ? ListView.view.height * 0.105 : 0
                color: ListView.isCurrentItem
                    ? Palette.secondaryContainer
                    : (rowHover.hovered
                        ? Palette.surfaceContainerHigh
                        : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest))

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
                }

                // UI-12: counts share the right-aligned column, like "N tracks"
                // in the album list.
                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: row.viewWidth * 0.02
                    text: row.albumCount + " albums"
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
                        root.artistActivated(row.artistId, row.artistName, row.cover);
                    }
                }
            }
        }
    }
}
