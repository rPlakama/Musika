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

// All indexed songs. Enter (or click) plays the selected song.
Rectangle {
    id: root

    color: Palette.surface

    ListModel {
        id: songs
    }

    function formatTime(seconds) {
        var total = Math.max(0, Math.floor(seconds || 0));
        return Math.floor(total / 60) + ":" + ("0" + (total % 60)).slice(-2);
    }

    function reload() {
        Backend.requestSongs();
    }

    function apply(payload) {
        songs.clear();
        var list = JSON.parse(payload);
        for (var i = 0; i < list.length; i++) {
            var song = list[i];
            songs.append({
                songId: song.id,
                title: song.title,
                artist: song.artist ? song.artist : "",
                album: song.album ? song.album : "",
                duration: formatTime(song.duration),
                cover: song.cover ? song.cover : ""
            });
        }
    }

    function activateCurrent() {
        var song = songs.get(list.currentIndex);
        if (song)
            Backend.playSong(song.songId);
    }

    function handleAction(action) {
        // Ctrl-N / Ctrl-P skip tracks (VI-10); j/k keeps the list cursor.
        if (action === "scroll_up") {
            Backend.playerPrevious();
            return;
        }
        if (action === "scroll_down") {
            Backend.playerNext();
            return;
        }

        if (songs.count === 0)
            return;
        if (action === "move_up")
            list.currentIndex = Math.max(0, list.currentIndex - 1);
        else if (action === "move_down")
            list.currentIndex = Math.min(songs.count - 1, list.currentIndex + 1);
        else if (action === "enter")
            activateCurrent();
        list.positionViewAtIndex(list.currentIndex, ListView.Contain);
    }

    Component.onCompleted: reload()

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "songs")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }

    FadeText {
        anchors.centerIn: parent
        visible: songs.count === 0
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
                text: "Songs"
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }
            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.right: parent.right
                anchors.rightMargin: root.width * 0.02
                text: songs.count + " indexed"
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.021
            }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: songs
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: row
                required property int index
                required property string songId
                required property string title
                required property string artist
                required property string album
                required property string duration
                required property string cover

                readonly property real viewWidth: ListView.view ? ListView.view.width : 0

                width: viewWidth
                height: ListView.view ? ListView.view.height * 0.09 : 0
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
                    width: row.height * 0.72
                    height: row.height * 0.72
                    radius: Theme.radius
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
                    anchors.leftMargin: row.viewWidth * 0.02 + row.height * 0.72 + row.viewWidth * 0.02
                    spacing: row.height * 0.05

                    FadeText {
                        text: row.title
                        color: Palette.onSurface
                        font.pixelSize: row.height * 0.26
                        animated: false
                    }
                    FadeText {
                        text: row.artist + (row.album.length > 0 ? "  ·  " + row.album : "")
                        color: Palette.onSurfaceVariant
                        font.pixelSize: row.height * 0.2
                        animated: false
                    }
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: row.viewWidth * 0.02
                    text: row.duration
                    color: Palette.onSurfaceVariant
                    font.pixelSize: row.height * 0.2
                    animated: false
                }

                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        list.currentIndex = row.index;
                        Backend.playSong(row.songId);
                    }
                }
            }
        }
    }
}
