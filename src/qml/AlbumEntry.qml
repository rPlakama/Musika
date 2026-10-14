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
import QtQuick.Effects
import musika
import "Theme.js" as Theme

// Album entry: hero cover beside the album description, then its songs.
// The art doubles as the ambience: a blurred copy fades into the theme
// surface behind the header (roadmap D-5).
//
// Set `albumId` to load the real track list from library.db; clicking a song
// queues the album at that track. The descriptive fields are still placeholders
// until album metadata is passed in.
Rectangle {
    id: root

    signal back

    property string albumId: ""
    property bool onToolbar: false
    property string albumCover: ""
    property string albumTitle: ""
    property string albumArtist: ""
    property string albumMeta: ""
    property string albumNotes: ""
    // UI-14: album notes clamp to three lines until clicked.
    property bool notesExpanded: false

    // Header geometry (roadmap "Album page cover" D-3): the hero cover is
    // sized from the pane height, beside the description when there is room
    // and stacked over it on narrow panes.
    readonly property real headerHeight: Math.min(height * 0.44, 512 + height * 0.02)
    readonly property real coverSide: Math.max(160, Math.min(height * 0.42, 512))
    readonly property bool sideBySide: width >= 900

    color: Palette.surface

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
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
                duration: Theme.formatTime(song.duration)
            });
        }
    }

    Component.onCompleted: reload()

    function handleAction(action) {
        // VI-11: `gg`/`G` leave the toolbar for the first/last song; the
        // toolbar stays a `k`-from-row-0 affordance only.
        if (action === "top" || action === "bottom") {
            if (songs.count === 0)
                return;
            root.onToolbar = false;
            Theme.cursorEdge(songList, action === "bottom", ListView.Contain);
            return;
        }

        if (root.onToolbar) {
            // Cursor is on the toolbar (Back). Down returns to the songs.
            if (action === "move_down" && songs.count > 0) {
                root.onToolbar = false;
                songList.currentIndex = 0;
                songList.positionViewAtIndex(0, ListView.Contain);
            } else if (action === "enter") {
                root.back();
            }
            return;
        }

        if (action === "move_up") {
            if (songList.currentIndex > 0)
                songList.currentIndex -= 1;
            else
                root.onToolbar = true;
        } else if (action === "move_down") {
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

    // Ambience (D-5): the album art itself through the shared
    // `CoverImage.ambient` mode — the GPU layer blurs and desaturates it, then
    // the gradient below washes it into the theme surface. This is the one
    // non-modal blur: content-derived ambience, never elevation (R2 amended).
    CoverImage {
        id: coverBackdrop
        anchors.fill: parent
        source: root.albumCover
        ambient: true
    }

    Rectangle {
        anchors.fill: parent
        visible: coverBackdrop.visible
        gradient: Gradient {
            GradientStop {
                position: 0.0
                color: root.withAlpha(Palette.surface, 0.82)
            }
            GradientStop {
                position: 0.38
                color: root.withAlpha(Palette.surface, 0.30)
            }
            GradientStop {
                position: 1.0
                color: root.withAlpha(Palette.surface, 1.0)
            }
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
                implicitWidth: backContent.implicitWidth + root.width * 0.04
                implicitHeight: root.height * 0.045
                radius: Theme.radius
                color: (root.onToolbar || backHover.hovered) ? Palette.secondaryContainer : Palette.surfaceContainerHigh
                border.color: root.onToolbar ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                // "<icon> ESC" (SymbolIcon: white-baked asset, re-coloured to
                // the fixed strong glyph tone; fixed frame from the button).
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

        // Header (D-3): cover hero + description. The grid collapses to one
        // column on narrow panes, which stacks the description under the
        // centered cover without a second layout.
        GridLayout {
            id: header
            Layout.fillWidth: true
            Layout.preferredHeight: root.sideBySide ? root.headerHeight : implicitHeight
            columns: root.sideBySide ? 2 : 1
            columnSpacing: root.width * 0.025
            rowSpacing: root.height * 0.012

            Rectangle {
                id: coverBox
                Layout.preferredWidth: root.coverSide
                Layout.preferredHeight: root.coverSide
                Layout.alignment: root.sideBySide ? (Qt.AlignLeft | Qt.AlignVCenter) : Qt.AlignHCenter
                radius: Theme.radius
                color: Palette.primaryContainer
                clip: true

                CoverImage {
                    id: coverImage
                    anchors.fill: parent
                    source: root.albumCover
                }
            }

            // Description.
            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignVCenter
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
                    id: notesText
                    Layout.fillWidth: true
                    Layout.topMargin: root.height * 0.008
                    visible: root.albumNotes.length > 0
                    text: root.albumNotes
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                    lineHeight: 1.25
                    wrapMode: Text.WordWrap
                    // UI-14: clamped to three lines; click expands.
                    maximumLineCount: root.notesExpanded ? 1000 : 3
                    elide: root.notesExpanded ? Text.ElideNone : Text.ElideRight

                    HoverHandler {
                        enabled: root.albumNotes.length > 0
                        cursorShape: Qt.PointingHandCursor
                    }
                    TapHandler {
                        enabled: root.albumNotes.length > 0
                        onTapped: root.notesExpanded = !root.notesExpanded
                    }
                }
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
                color: (ListView.isCurrentItem && !root.onToolbar) ? Palette.secondaryContainer : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
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

                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
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
