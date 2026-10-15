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
    /// ARC-I-2: Alt-hold quick actions on the song row under the cursor;
    /// Home routes it to the shared overlay.
    signal quickAction(var target, var actions)

    property string albumId: ""
    property bool onToolbar: false
    property string albumCover: ""
    property string albumTitle: ""
    property string albumArtist: ""
    property string albumMeta: ""
    /// ARC-I-2: the album's favourite state (header heart); fed by the
    /// opener from the album row and the ARC-43 page payload.
    property bool albumStarred: false
    property string albumNotes: ""
    // UI-14: album notes clamp to three lines until clicked.
    property bool notesExpanded: false
    // UI-22: true while the album's songs are in flight.
    property bool loading: false
    // UI-54: per-album scroll memory for the songs viewport.
    readonly property string memoryKey: "album:" + root.albumId
    property bool _restored: false

    // Header geometry (roadmap "Album page cover" D-3): the hero cover is
    // sized from the pane height, beside the description when there is room
    // and stacked over it on narrow panes.
    readonly property real headerHeight: Math.min(height * 0.44, 512 + height * 0.02)
    readonly property real coverSide: Math.max(160, Math.min(height * 0.42, 512))
    readonly property bool sideBySide: width >= Theme.breakpoints.compact

    // The compact flip reflows the header: restart the cover's dip-and-blur
    // (below) so the reflow does not read as a flick.
    onSideBySideChanged: {
        if (!Theme.reduceMotion)
            coverAppear.restart();
    }

    color: Palette.surface

    // A color with a replaced alpha, for the ambience gradient stops.
    function withAlpha(color, alpha) {
        return Qt.rgba(color.r, color.g, color.b, alpha);
    }

    ListModel {
        id: songs
    }

    // ARC-I-2: the recycled Alt verb set; album songs are tracks.
    QuickActions {
        id: verbs
    }

    /// VI-4: the selection state lives in `ScrollContent` below; this page
    /// owns only its cursor/toolbar semantics.
    readonly property bool hasSelection: content.hasSelection
    readonly property int selectionCount: content.selectionCount

    /// VI-4: `v` toggles visual mode on the cursor row (the toolbar owns no
    /// selection).
    function toggleSelection() {
        if (root.onToolbar)
            return;
        content.toggleSelection();
    }

    /// VI-4: `V` selects the whole track list.
    function selectAll() {
        if (songs.count === 0)
            return;
        root.onToolbar = false;
        content.currentIndex = songs.count - 1;
        content.selectAllRows();
        content.ensureVisible();
    }

    function clearSelection() {
        content.clearSelection();
    }

    /// ARC-I-2: Alt armed the gesture — anchor on the song row under the
    /// cursor (the toolbar is a no-op, like an empty list); a visual selection
    /// swaps in the batch verb set (VI-4).
    function armQuickActions() {
        if (root.onToolbar || songs.count === 0 || content.currentIndex < 0)
            return;
        var row = content.view.itemAtIndex(content.currentIndex);
        var actions = root.hasSelection
            ? verbs.forSelection("song", content.selectionIds("songId"))
            : (songs.get(content.currentIndex) ? verbs.song(songs.get(content.currentIndex)) : null);
        if (actions && row)
            quickAction(row, actions);
    }

    function reload() {
        if (root.albumId === "")
            return;
        root.loading = true;
        Backend.requestAlbumSongs(root.albumId);
        Backend.requestAlbumInfo(root.albumId);
    }

    function applyInfo(payload) {
        var info = JSON.parse(payload);
        root.albumNotes = info.notes ? info.notes : "";
    }

    function apply(payload) {
        // VI-4: a reload invalidates the selected range.
        content.clearSelection();
        songs.clear();
        var list = JSON.parse(payload);
        for (var i = 0; i < list.length; i++) {
            var song = list[i];
            songs.append({
                track: song.track ? String(song.track) : String(i + 1),
                title: song.title,
                duration: Theme.formatTime(song.duration),
                starred: song.starred === true,
                songId: song.id,
                albumId: song.album_id ? song.album_id : ""
            });
        }
        root.loading = false;
        // UI-54: reopening the page restores the cursor + offset; later
        // reloads (library change, resync) keep the live state.
        if (!root._restored) {
            root._restored = true;
            content.restore();
        }
    }

    Component.onCompleted: reload()

    /// ARC-UI-2 M1: the search contract (see `ScrollContent`).
    property alias searchQuery: content.searchQuery
    property alias searchCurrentRow: content.searchCurrentRow

    /// UI-58: the song-row baseline (0.055 of the page), captured on the
    /// first real measure so the adaptive rule preserves the default size.
    property real _rowBase: 0
    onHeightChanged: if (_rowBase <= 0 && height > 0) _rowBase = height * 0.055

    function searchRows() {
        return content.searchRows();
    }

    function searchJump(row) {
        content.searchJump(row);
    }

    function handleAction(action) {
        // VI-11: `gg`/`G` leave the toolbar for the first/last song; the
        // toolbar stays a `k`-from-row-0 affordance only.
        if (action === "top" || action === "bottom") {
            if (songs.count === 0)
                return;
            root.onToolbar = false;
            Theme.cursorEdge(content.view, action === "bottom", ListView.Contain);
            return;
        }

        if (root.onToolbar) {
            // Cursor is on the toolbar (Back). Down returns to the songs.
            if (action === "move_down" && songs.count > 0) {
                root.onToolbar = false;
                // The view follows the cursor itself.
                content.currentIndex = 0;
            } else if (action === "enter") {
                root.back();
            }
            return;
        }

        if (action === "move_up") {
            if (content.currentIndex > 0) {
                content.currentIndex -= 1;
            } else {
                // VI-4: leaving the rows for the toolbar ends the selection.
                content.clearSelection();
                root.onToolbar = true;
            }
        } else if (action === "move_down") {
            if (songs.count > 0)
                content.currentIndex = Math.min(songs.count - 1, content.currentIndex + 1);
        } else if (action === "enter") {
            content.clearSelection();
            Backend.playAlbum(root.albumId, content.currentIndex);
        }
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

            ExitChip {
                host: root
                label: "ESC/Q"
                highlighted: root.onToolbar
                onClicked: root.back()
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

                // The reflow FX: the offscreen layer only exists while the
                // blur is up.
                layer.enabled: coverBlur.blur > 0.001
                layer.effect: MultiEffect {
                    id: coverBlur
                    blurEnabled: true
                    blur: 0
                    blurMax: 48
                    autoPaddingEnabled: false
                }

                // Crossing the compact breakpoint reflows the hero: dim it to
                // 70% and blur it in over 200 ms, then restore both over the
                // emphasized tier — the reflow hides under the dip.
                SequentialAnimation {
                    id: coverAppear
                    ParallelAnimation {
                        NumberAnimation {
                            target: coverBox
                            property: "opacity"
                            to: 0.7
                            duration: 200
                        }
                        NumberAnimation {
                            target: coverBlur
                            property: "blur"
                            to: 0.5
                            duration: 200
                        }
                    }
                    ParallelAnimation {
                        NumberAnimation {
                            target: coverBox
                            property: "opacity"
                            to: 1
                            duration: Theme.motion.emphasized
                            easing.type: Easing.Bezier
                            easing.bezierCurve: Theme.ease.decelerate
                        }
                        NumberAnimation {
                            target: coverBlur
                            property: "blur"
                            to: 0
                            duration: Theme.motion.emphasized
                            easing.type: Easing.Bezier
                            easing.bezierCurve: Theme.ease.decelerate
                        }
                    }
                }

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
                RowLayout {
                    Layout.fillWidth: true
                    spacing: root.height * 0.008

                    // ARC-I-2: the album's favourite leads the header meta.
                    Icon {
                        visible: root.albumStarred
                        Layout.alignment: Qt.AlignVCenter
                        Layout.preferredWidth: root.height * 0.016
                        Layout.preferredHeight: width
                        name: "baseline-favorite"
                        symbolColor: Theme.iconStrong
                    }

                    FadeText {
                        Layout.fillWidth: true
                        text: root.albumMeta
                        color: Palette.onSurfaceVariant
                        font.pixelSize: root.height * 0.02
                        elide: Text.ElideRight
                    }
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
        RowLayout {
            Layout.fillWidth: true
            spacing: root.height * 0.012

            FadeText {
                text: "Songs"
                color: Palette.primary
                font.pixelSize: root.height * 0.024
                font.bold: true
                font.letterSpacing: root.height * 0.004
            }

            Item {
                Layout.fillWidth: true
            }

            // VI-4: the selection count pill.
            SelectionBadge {
                Layout.alignment: Qt.AlignVCenter
                count: root.selectionCount
                pixelSize: root.height * 0.02
            }
        }

        ScrollContent {
            id: content
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: songs
            selectable: true
            memoryKey: root.memoryKey
            visible: !(root.loading && songs.count === 0)
            // ARC-UI-2 M1: the rows paint track, title and duration.
            searchFields: ["track", "title", "duration"]

            rowDelegate: Component {
                Rectangle {
                    id: songRow
                required property int index
                required property string track
                required property string title
                required property string duration
                required property bool starred

                // VI-4: a row inside the visual selection wears the cursor fill.
                readonly property bool selectedRow: content.selected(index)

                width: ListView.view.width
                // UI-58: adaptive — cut a row past the 30% squeeze.
                height: Theme.adaptiveRow(root.height, 0.055, root._rowBase)
                color: ((ListView.isCurrentItem && !root.onToolbar) || songRow.selectedRow)
                    ? Palette.secondaryContainer
                    : (index % 2 === 0 ? Palette.surface : Palette.surfaceContainerLowest)

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                SearchText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    width: songRow.height * 0.6
                    text: songRow.track
                    color: Palette.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                    horizontalAlignment: Text.AlignRight
                    animated: false
                    query: content.searchQuery
                    currentMatch: songRow.index === content.searchCurrentRow
                }
                SearchText {
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
                    query: content.searchQuery
                    currentMatch: songRow.index === content.searchCurrentRow
                }
                SearchText {
                    id: songDuration
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: root.width * 0.02
                    text: songRow.duration
                    color: Palette.onSurfaceVariant
                    font.pixelSize: songRow.height * 0.32
                    animated: false
                    query: content.searchQuery
                    currentMatch: songRow.index === content.searchCurrentRow
                }

                // UI-56: the favourite heart leads the duration.
                Icon {
                    visible: songRow.starred
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: songDuration.left
                    anchors.rightMargin: songRow.height * 0.12
                    width: songDuration.font.pixelSize * 0.8
                    height: width
                    name: "baseline-favorite"
                    symbolColor: Theme.iconStrong
                }

                HoverHandler {
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    onTapped: {
                        // VI-4: direct play ends the visual selection.
                        content.clearSelection();
                        content.currentIndex = songRow.index;
                        Backend.playAlbum(root.albumId, songRow.index);
                    }
                }
                }
            }
        }

        // UI-22: the songs area has a wait state and a real empty-album state.
        SkeletonList {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: root.loading && songs.count === 0
        }

        FadeText {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !root.loading && songs.count === 0
            text: "No songs in this album"
            color: Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("body", root.height)
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
    }

}
