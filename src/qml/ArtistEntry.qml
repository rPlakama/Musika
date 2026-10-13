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

// Artist entry (P-8, milestone A): round avatar + name over a four-column
// album grid with titles under the covers. Pushed from ArtistList and from
// artist hits in search; Enter on a cover opens the album page, Enter/Back
// from here always returns to Main (see Home.leaveArtistEntry). Favorite,
// stars, biography and the cache rule land in milestones B-D.
Rectangle {
    id: root

    signal back
    signal albumActivated(string albumId, string title, string artist, string year, string tracks, string cover)

    property string artistId: ""
    property string artistName: ""
    property string artistCover: ""
    property string artistMeta: ""

    // Home uses this to send `Esc` straight to Main (P-8) instead of popping
    // back to the Artists segment or a search result.
    readonly property bool exitsToMain: true

    readonly property int columns: 4
    readonly property real avatarSide: Math.max(96, Math.min(height * 0.22, 256))
    readonly property real cellWidth: width / columns
    readonly property real coverSide: cellWidth * 0.78
    readonly property real cellHeight: coverSide + height * 0.085

    // Study §2 "name+server tag": host only, never the full URL.
    readonly property string serverTag: {
        var url = Backend.serverUrl;
        var mark = url.indexOf("://");
        var rest = mark >= 0 ? url.substring(mark + 3) : url;
        var slash = rest.indexOf("/");
        return slash >= 0 ? rest.substring(0, slash) : rest;
    }

    color: Palette.surface

    ListModel {
        id: albums
    }

    function reload() {
        if (root.artistId === "")
            return;
        Backend.requestArtist(root.artistId);
    }

    function apply(payload) {
        var artist = JSON.parse(payload);
        if (artist.name && artist.name.length > 0)
            root.artistName = artist.name;
        if (artist.cover && artist.cover.length > 0)
            root.artistCover = artist.cover;
        var count = Number(artist.album_count);
        root.artistMeta = count + (count === 1 ? " album" : " albums");

        albums.clear();
        var rows = artist.albums ? artist.albums : [];
        for (var i = 0; i < rows.length; i++) {
            var album = rows[i];
            var meta = [];
            if (album.year)
                meta.push(String(album.year));
            meta.push(album.song_count + (album.song_count === 1 ? " song" : " songs"));
            albums.append({
                albumId: album.id,
                title: album.title,
                artist: album.artist ? album.artist : "",
                year: album.year ? String(album.year) : "",
                tracks: String(album.song_count),
                meta: meta.join("   ·   "),
                cover: album.cover ? album.cover : ""
            });
        }
        if (albums.count > 0 && (grid.currentIndex < 0 || grid.currentIndex >= albums.count))
            grid.currentIndex = 0;
    }

    function activateCurrent() {
        var album = albums.get(grid.currentIndex);
        if (album)
            root.albumActivated(album.albumId, album.title, album.artist, album.year, album.tracks, album.cover);
    }

    Component.onCompleted: reload()

    // Movement and Enter belong to the cover grid; `Esc` is handled by Home
    // (leaves the entry for Main regardless of where it was opened from).
    function handleAction(action) {
        if (albums.count === 0)
            return;
        if (action === "move_left")
            grid.moveCurrentIndexLeft();
        else if (action === "move_right")
            grid.moveCurrentIndexRight();
        else if (action === "move_up")
            grid.moveCurrentIndexUp();
        else if (action === "move_down")
            grid.moveCurrentIndexDown();
        else if (action === "enter")
            activateCurrent();
        grid.positionViewAtIndex(grid.currentIndex, GridView.Contain);
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "artist")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
        function onAlbum_order_changed() {
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
                implicitWidth: backContent.implicitWidth + root.width * 0.04
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: backHover.hovered ? Palette.secondaryContainer : Palette.surfaceContainerHigh
                border.color: Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                Row {
                    id: backContent
                    anchors.centerIn: parent
                    spacing: root.height * 0.01

                    Item {
                        anchors.verticalCenter: parent.verticalCenter
                        width: root.height * 0.026
                        height: width

                        SymbolIcon {
                            anchors.centerIn: parent
                            source: "qrc:/app/icons/exit-2-svgrepo-com.svg"
                            symbolColor: Theme.iconStrong
                            width: parent.width * 0.85
                            height: width
                        }
                    }

                    FadeText {
                        anchors.verticalCenter: parent.verticalCenter
                        text: "ESC"
                        color: Palette.onSurface
                        font.pixelSize: root.height * 0.022
                    }
                }

                HoverHandler {
                    id: backHover
                    cursorShape: Qt.PointingHandCursor
                }
                TapHandler {
                    onTapped: root.back()
                }
            }

            Item {
                Layout.fillWidth: true
            }
        }

        // Header: the only circle in the product is the artist avatar (R3).
        RowLayout {
            Layout.fillWidth: true
            spacing: root.width * 0.025

            Rectangle {
                Layout.preferredWidth: root.avatarSide
                Layout.preferredHeight: root.avatarSide
                Layout.alignment: Qt.AlignVCenter
                radius: width * 0.5
                color: Palette.primaryContainer
                clip: true

                CoverImage {
                    anchors.fill: parent
                    source: root.artistCover
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
                spacing: root.height * 0.006

                FadeText {
                    Layout.fillWidth: true
                    text: root.artistName
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.036
                    font.bold: true
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: root.artistMeta + (root.serverTag.length > 0 ? "   ·   " + root.serverTag : "")
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                    elide: Text.ElideRight
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Palette.outlineVariant
        }

        // Albums: four covers per row, title and year/count underneath.
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            GridView {
                id: grid
                anchors.fill: parent
                clip: true
                model: albums
                cellWidth: width / root.columns
                cellHeight: root.cellHeight
                boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                id: cell
                required property int index
                required property string albumId
                required property string title
                required property string artist
                required property string year
                required property string tracks
                required property string meta
                required property string cover

                readonly property bool current: GridView.isCurrentItem

                width: GridView.view.cellWidth
                height: GridView.view.cellHeight
                color: "transparent"

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: cell.width * 0.03
                    radius: Theme.radius
                    color: cell.current
                        ? Palette.secondaryContainer
                        : (cellHover.hovered ? Palette.surfaceContainerHigh : "transparent")

                    Behavior on color {
                        ColorAnimation {
                            duration: Theme.motion.quick
                            easing.type: Easing.Bezier
                            easing.bezierCurve: Theme.ease.standard
                        }
                    }
                }

                Rectangle {
                    id: coverBox
                    width: root.coverSide
                    height: width
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.top: parent.top
                    anchors.topMargin: cell.height * 0.05
                    radius: Theme.radius
                    color: Palette.surfaceContainerLowest
                    border.color: cell.current ? Palette.primary : "transparent"
                    border.width: Math.max(1, cell.height * Theme.hairlineRatio)
                    clip: true

                    CoverImage {
                        anchors.fill: parent
                        source: cell.cover
                    }
                }

                FadeText {
                    id: titleText
                    anchors.top: coverBox.bottom
                    anchors.topMargin: cell.height * 0.025
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: parent.width * 0.86
                    text: cell.title
                    color: Palette.onSurface
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    font.pixelSize: Math.max(12, cell.height * 0.105)
                    animated: false
                }

                FadeText {
                    anchors.top: titleText.bottom
                    anchors.topMargin: cell.height * 0.015
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: parent.width * 0.86
                    text: cell.meta
                    color: Palette.onSurfaceVariant
                    horizontalAlignment: Text.AlignHCenter
                    elide: Text.ElideRight
                    font.pixelSize: Math.max(10, cell.height * 0.08)
                    animated: false
                }

                HoverHandler {
                    id: cellHover
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        grid.currentIndex = cell.index;
                        root.activateCurrent();
                    }
                }
            }
        }

        FadeText {
            anchors.centerIn: parent
            visible: albums.count === 0
            text: "No albums indexed yet"
            color: Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("body", root.height)
        }
    }
}

}
