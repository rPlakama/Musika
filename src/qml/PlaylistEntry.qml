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

// ARC-44 M2: the playlist page. Same identity-over-artwork/tracklist shape as
// AlbumEntry, with a golden-ratio collage of the playlist's song covers as its
// ambience. Unlike the Playlists tab, every track row keeps its own cover.
Rectangle {
    id: root
    radius: Theme.radius

    RowMetrics {
        id: rowMetrics
        viewWidth: content.width
        rowHeight: content.adaptiveRowHeight
    }

    signal back

    property string playlistId: ""
    property string playlistTitle: ""
    property string playlistSubtitle: ""
    property string playlistMeta: ""
    property var collage: []
    property bool loading: false
    property string _loadingId: ""

    readonly property string memoryKey: "playlist:" + root.playlistId
    readonly property bool hasSelection: content.hasSelection
    readonly property int selectionCount: content.selectionCount
    readonly property int currentIndex: content.currentIndex

    color: Palette.surface

    ListModel {
        id: songs
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "playlist")
                root.apply(payload);
        }
        function onLibrary_changed() {
            root.reload();
        }
    }

    function reload() {
        if (root.playlistId === "")
            return;
        if (root._loadingId === root.playlistId)
            return;
        root._loadingId = root.playlistId;
        root.loading = songs.count === 0;
        Backend.requestPlaylist(root.playlistId);
    }

    function apply(payload) {
        var data = JSON.parse(payload);
        if (data.playlist_id !== root.playlistId)
            return;
        root._loadingId = "";
        root.playlistTitle = data.name ? data.name : "";
        root.playlistSubtitle = data.subtitle ? data.subtitle : "";
        root.playlistMeta = data.meta ? data.meta : "";
        root.collage = data.collage ? data.collage : [];
        songs.clear();
        var rows = data.songs ? data.songs : [];
        for (var i = 0; i < rows.length; i++) {
            var row = rows[i];
            songs.append({
                track: row.track ? String(row.track) : String(i + 1),
                title: row.title,
                subtitle: row.subtitle ? String(row.subtitle) : "",
                duration: row.duration ? String(row.duration) : "",
                cover: row.cover ? String(row.cover) : "",
                songId: row.songId ? String(row.songId) : "",
                albumId: row.albumId ? String(row.albumId) : "",
                starred: row.starred === true,
                playable: row.playable === true
            });
        }
        root.loading = false;
        content.clearSelection();
        if (songs.count > 0)
            content.land(0);
    }

    function playRow(index) {
        if (index < 0 || index >= songs.count)
            return;
        var row = songs.get(index);
        if (!row || row.playable !== true)
            return;
        Backend.playPlaylist(root.playlistId, index);
    }

    function yankRows(rowOnly) {
        if (songs.count === 0 || content.currentIndex < 0)
            return;
        var indices = (rowOnly || !root.hasSelection)
            ? [content.currentIndex] : content.selectionIndices();
        var ids = [];
        for (var i = 0; i < indices.length; ++i) {
            var row = songs.get(indices[i]);
            if (row && row.songId)
                ids.push(row.songId);
        }
        if (ids.length === 0)
            return;
        content.clearSelection();
        Backend.yankRows("song", JSON.stringify(ids));
    }

    function toggleSelection() {
        content.toggleSelection();
    }

    function selectAll() {
        if (songs.count === 0)
            return;
        content.currentIndex = songs.count - 1;
        content.selectAllRows();
        content.goto();
    }

    function clearSelection() {
        content.clearSelection();
    }

    function handleAction(action) {
        if (action === "top" || action === "bottom") {
            if (songs.count === 0)
                return;
            content.edgeJump(action === "bottom");
            return;
        }
        if (action === "page_up" || action === "page_down") {
            if (songs.count === 0)
                return;
            content.pageJump(action === "page_down");
            return;
        }
        if (action.indexOf("extend_") === 0) {
            if (songs.count === 0)
                return;
            content.extendSelection("move_" + action.substring(7));
            return;
        }
        if (action === "move_up") {
            if (content.currentIndex > 0)
                content.currentIndex -= 1;
            else
                content.clearSelection();
            return;
        }
        if (action === "move_down") {
            if (songs.count > 0)
                content.currentIndex = Math.min(songs.count - 1, content.currentIndex + 1);
            return;
        }
        if (action === "enter") {
            if (content.currentIndex >= 0) {
                content.clearSelection();
                root.playRow(content.currentIndex);
            }
            return;
        }
    }

    PlaylistCollage {
        anchors.fill: parent
        sources: root.collage
        maxSide: (root.height - root.width * 0.04) * 0.75
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.02
        anchors.bottomMargin: 0
        spacing: root.height * 0.02

        Rectangle {
            id: headerPanel
            Layout.alignment: Qt.AlignLeft
            Layout.fillWidth: false
            implicitWidth: Math.min(headerContent.implicitWidth + root.width * 0.02, root.width * 0.96)
            implicitHeight: headerContent.implicitHeight + root.width * 0.02
            Layout.preferredWidth: implicitWidth
            Layout.maximumWidth: root.width * 0.96
            Layout.preferredHeight: implicitHeight
            color: Qt.rgba(0, 0, 0, 0.4)
            radius: Theme.radius

            ColumnLayout {
                id: headerContent
                anchors.fill: parent
                anchors.margins: root.width * 0.01
                spacing: root.height * 0.006

                FadeText {
                    Layout.fillWidth: true
                    text: root.playlistTitle
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.04
                    font.bold: true
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: root.playlistSubtitle
                    color: Palette.primary
                    font.pixelSize: root.height * 0.024
                    elide: Text.ElideRight
                }
                FadeText {
                    Layout.fillWidth: true
                    text: root.playlistMeta
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                    elide: Text.ElideRight
                }
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: root.height * 0.008

            ScrollContent {
                id: content
                Layout.fillWidth: true
                Layout.fillHeight: true
                model: songs
                selectable: true
                memoryKey: root.memoryKey
                searchFields: ["track", "title", "subtitle", "duration"]
                visible: !(root.loading && songs.count === 0)
                rowRefHeight: root.height
                onRowTapped: (index, shift) => {
                    if (shift) {
                        content.extendSelectionTo(index);
                        return;
                    }
                    content.clearSelection();
                    content.currentIndex = index;
                    root.playRow(index);
                }

                rowDelegate: Component {
                    Rectangle {
                        id: songRow
                        radius: Theme.radius
                        required property int index
                        required property string track
                        required property string title
                        required property string subtitle
                        required property string duration
                        required property string cover
                        required property string songId
                        required property string albumId
                        required property bool starred
                        required property bool playable

                        readonly property bool selectedRow: content.selected(index)
                        readonly property bool currentItem: ListView.isCurrentItem
                        readonly property bool emphasized: ListView.isCurrentItem
                            || songRow.index === content.hoveredIndex

                        width: ListView.view ? ListView.view.width : 0
                        z: emphasized ? 1 : 0
                        height: content.adaptiveRowHeight
                        color: "transparent"
                        opacity: songRow.playable ? 1 : 0.45

                        RowHighlight {
                            index: songRow.index
                            striped: true
                            baseColor: Palette.surface
                            current: songRow.currentItem
                            hovered: songRow.index === content.hoveredIndex
                            selected: songRow.selectedRow
                        }

                        RowBody {
                            anchors.fill: parent
                            viewWidth: songRow.width
                            rowHeight: songRow.height
                            metrics: rowMetrics
                            emphasized: songRow.emphasized
                            current: songRow.currentItem
                            showCover: true
                            cover: songRow.cover
                            showSubtitle: songRow.subtitle.length > 0
                            title: songRow.track.length > 0
                                ? songRow.track + ". " + songRow.title : songRow.title
                            subtitle: songRow.subtitle
                            meta: songRow.duration
                            reserveMeta: true
                            metaAlignRight: true
                            leftMargin: songRow.width * 0.02
                            textGap: songRow.width * 0.02
                            rightMargin: songRow.width * 0.02
                            marksMargin: songRow.width * 0.01275
                            markCursor: content.currentIndex
                            visibleStart: content.firstVisibleIndex
                            visibleEnd: content.lastVisibleIndex
                            markIndex: songRow.index
                            markSongId: songRow.songId
                            markAlbumId: songRow.albumId
                            markStarred: songRow.starred
                            markRatingVisible: false
                            searchQuery: content.searchQuery
                            searchCurrent: songRow.index === content.searchCurrentRow
                        }
                    }
                }
            }

            SkeletonList {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: root.loading && songs.count === 0
            }

            FadeText {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: !root.loading && songs.count === 0
                text: "No songs in this playlist"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("body", root.height)
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
        }
    }

    Component.onCompleted: reload()
}
