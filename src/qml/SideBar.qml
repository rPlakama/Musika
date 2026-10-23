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

// Left sidebar: the library segments. `selected` is this region's cursor and is
// only moved while the sidebar is the active region. It stays collapsed to an
// icon rail: same segments, same cursor, labels and stats out of the way.
Rectangle {
    radius: Theme.radius
    id: root

    signal segmentActivated(int index)

    /// Icon-only rail (Home keeps this true while the sidebar is collapsed).
    property bool narrow: false

    property int selected: 0
    // UI-6: Playlists/Radio are placeholders — visibly disabled, not fake-active.
    readonly property var segments: [
        { label: "Albums", enabled: true, icon: "qrc:/app/icons/album.svg" },
        { label: "Songs", enabled: true, icon: "qrc:/app/icons/track-svgrepo-com.svg" },
        { label: "Artists", enabled: true, icon: "qrc:/app/icons/person-walking-svgrepo-com.svg" },
        { label: "Playlists", enabled: true, icon: "qrc:/app/icons/playlist-svgrepo-com.svg" },
        { label: "Radio", enabled: false, icon: "qrc:/app/icons/radio-signal-1026-svgrepo-com.svg" }
    ]

    color: Palette.surfaceContainerLow

    // UI-58: the row breathing ramp — driven by the window width, 1 at
    // `compact` and above, rising to 1.4 at the `narrow` icon strip.
    readonly property real _winWidth: Window.window ? Window.window.width : root.width
    readonly property real rowBreathe: {
        var s = Theme.widthScale(root._winWidth);
        var t = Math.max(0, Math.min(1, (1 - s) / 0.3));
        return 1 + 0.4 * t;
    }

    function moveSelection(delta) {
        var next = selected;
        for (var step = 0; step < segments.length; step++) {
            next = (next + delta + segments.length) % segments.length;
            if (segments[next].enabled)
                break;
        }
        selected = next;
    }

    function handleAction(action) {
        if (action === "move_up")
            moveSelection(-1);
        else if (action === "move_down")
            moveSelection(1);
        else if (action === "page_up")
            moveSelection(-5);
        else if (action === "page_down")
            moveSelection(5);
        else if (action === "enter" && segments[selected].enabled)
            segmentActivated(selected);
    }

    /// ARC-UI-2: the search contract — one row per segment, aligned with the
    /// cursor. A disabled segment renders nothing searchable (empty cell), so
    /// it never matches and never affords a dead jump; a jump moves the
    /// cursor, never activates (the study's rule).
    function searchRows() {
        var out = [];
        for (var i = 0; i < segments.length; ++i)
            out.push(segments[i].enabled ? [segments[i].label] : [""]);
        return out;
    }

    function searchJump(row) {
        if (row < 0 || row >= segments.length || !segments[row].enabled)
            return;
        selected = row;
    }

    // Brand: the logo's original row, kept centered by its parent rectangle
    // at any width. The square side preserves aspect (never squashed); the
    // width cap keeps it clear of the pane walls.
    Rectangle {
        radius: Theme.radius
        id: brand
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: root.height * 0.09
        color: "transparent"

        SymbolIcon {
            anchors.centerIn: parent
            readonly property real side: Math.min(parent.height * 0.6, root.width * 0.5)
            width: side
            height: side
            source: "qrc:/app/icons/musika-glyph.svg"
            symbolColor: Theme.iconStrong
        }
    }

    ColumnLayout {
        anchors.top: brand.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: root.narrow ? root.width * 0.14 : root.width * 0.07
        spacing: root.height * 0.004

        Repeater {
            model: root.segments

            delegate: Rectangle {
                id: segment
                required property int index
                required property var modelData

                readonly property bool usable: modelData.enabled

                Layout.fillWidth: true
                // UI-58 (left bar): as the pane collapses by width the rows
                // breathe — up to 40% taller on a continuous ramp (1 at the
                // default width, where the spacing reads right), so the icon
                // strip stays legible.
                implicitHeight: root.height * 0.064 * root.rowBreathe
                radius: Theme.radius
                color: !usable
                    ? "transparent"
                    : (segment.index === root.selected
                        ? Palette.secondaryContainer
                        : (segmentHover.hovered ? Palette.surfaceContainerHigh : "transparent"))

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                }

                // Accent bar on the selected segment.
                Rectangle {
                    radius: Theme.radius
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.width * 0.02
                    color: Palette.primary
                    visible: segment.usable && segment.index === root.selected
                }

                FadeText {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: root.width * 0.1
                    visible: !root.narrow
                    text: segment.modelData.label
                    color: !segment.usable
                        ? Theme.stateDisabledContent
                        : (segment.index === root.selected ? Palette.onSecondaryContainer : Palette.onSurfaceVariant)
                    font.pixelSize: root.height * 0.022
                    font.bold: segment.usable && segment.index === root.selected
                }

                // UI-32 R2: the label becomes an icon in the narrow strip.
                SymbolIcon {
                    anchors.centerIn: parent
                    visible: root.narrow
                    width: Math.min(root.width * 0.5, segment.height * 0.5)
                    height: width
                    source: segment.modelData.icon
                    opacity: segment.index === root.selected ? 1.0 : 0.6
                    symbolColor: !segment.usable
                        ? Theme.stateDisabledContent
                        : (segment.index === root.selected ? Palette.onSecondaryContainer : Palette.onSurfaceVariant)
                }

                HoverHandler {
                    id: segmentHover
                    enabled: segment.usable
                    cursorShape: Qt.PointingHandCursor
                }

                TapHandler {
                    enabled: segment.usable
                    onTapped: {
                        root.selected = segment.index;
                        root.segmentActivated(segment.index);
                    }
                }
            }
        }
    }

    // Library quantity statistics (the sync action lives in the top bar).
    // Hidden in the narrow strip: no room for labels + values.
    ColumnLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: root.width * 0.1
        spacing: root.height * 0.012
        visible: !root.narrow

        Rectangle {
            radius: Theme.radius
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.height * Theme.hairlineRatio)
            color: Palette.outlineVariant
        }

        Repeater {
            model: [
                { label: "Artists", value: Backend.artistCount },
                { label: "Albums", value: Backend.albumCount },
                { label: "Songs", value: Backend.songCount }
            ]

            delegate: RowLayout {
                id: stat
                required property var modelData
                Layout.fillWidth: true

                FadeText {
                    Layout.fillWidth: true
                    text: stat.modelData.label
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.02
                }
                FadeText {
                    text: stat.modelData.value
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.02
                    font.bold: true
                }
            }
        }
    }
}
