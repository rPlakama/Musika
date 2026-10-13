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
import QtQuick.VectorImage
import musika
import "Theme.js" as Theme

// Top status bar: indexed counts, sync state/progress and a sync action.
Rectangle {
    id: root

    color: Palette.surfaceContainerLow

    // Human name for the ping envelope's `type` (`navidrome` → "Navidrome").
    function serverName() {
        if (Backend.serverType.length === 0)
            return "";
        return Backend.serverType.charAt(0).toUpperCase() + Backend.serverType.slice(1);
    }

    // Relative "last sync" label: Now / HH:mm / dd / Nw / Nmo / Ny.
    function lastSyncText() {
        var seconds = Number(Backend.lastSync);
        if (!Backend.lastSync || !seconds)
            return "—";

        var then = new Date(seconds * 1000);
        var diff = (Date.now() - then.getTime()) / 1000; // seconds ago
        if (diff < 60)
            return "Now";

        var hours = diff / 3600;
        if (hours < 24)
            return Qt.formatDateTime(then, "HH:mm");

        var days = hours / 24;
        if (days < 7)
            return Qt.formatDateTime(then, "dd");

        var weeks = days / 7;
        if (weeks < 4)
            return Math.floor(weeks) + "w";

        var months = days / 30;
        if (months < 12)
            return Math.floor(months) + "mo";

        return Math.floor(days / 365) + "y";
    }

    // The status panel has no cursor of its own; Enter triggers a sync.
    function handleAction(action) {
        if (action === "enter")
            Backend.syncLibrary();
    }

    // Bottom divider.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Math.max(1, root.height * Theme.hairlineRatio)
        color: Palette.outlineVariant
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.012
        anchors.rightMargin: root.width * 0.012
        spacing: root.width * 0.015

        // Mode chip (N4 placeholder): always shows the mode; NORMAL until a
        // global mode machine exists.
        Rectangle {
            implicitWidth: modeLabel.implicitWidth + root.height * 0.36
            implicitHeight: root.height * 0.5
            radius: Theme.radius
            color: Palette.surfaceContainerHighest
            border.color: Palette.primary
            border.width: Math.max(1, root.height * Theme.hairlineRatio)

            FadeText {
                id: modeLabel
                anchors.centerIn: parent
                text: "NORMAL"
                color: Palette.primary
                font.pixelSize: Theme.fontSize("label", root.height)
                font.bold: true
                font.letterSpacing: root.height * 0.025
                animated: false
            }
        }

        Item {
            Layout.fillWidth: true
        }

        // One status line: "Last Sync: <status>, <provider>".
        FadeText {
            text: {
                var status = Backend.librarySyncing ? "Syncing…" : root.lastSyncText();
                var provider = root.serverName();
                return "Last Sync: " + status + (provider.length > 0 ? ", " + provider : "");
            }
            color: Backend.librarySyncing ? Palette.tertiary : Palette.onSurfaceVariant
            font.pixelSize: Theme.fontSize("label", root.height)
        }

        // Determinate progress across the last reported stage. Collapsed to a
        // 2px tick when idle, expanding while a sync runs (the cluster is
        // right-anchored, so the bar grows/shrinks toward the sync icon).
        Rectangle {
            id: track
            Layout.preferredWidth: Backend.librarySyncing ? root.width * 0.12 : 2
            Layout.preferredHeight: Math.max(2, root.height * 0.16)
            color: Palette.outlineVariant
            clip: true

            Behavior on Layout.preferredWidth {
                NumberAnimation { duration: Theme.motion.standard; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
            }

            Rectangle {
                width: track.width * Math.max(0, Math.min(1, Backend.syncProgress))
                height: parent.height
                color: Backend.librarySyncing ? Palette.tertiary : Palette.primary

                Behavior on width {
                    NumberAnimation {
                        id: progressAnim
                        duration: Theme.motion.quick
                        easing.type: Easing.Linear
                        // Spin follows the bar: when the fill settles after the
                        // last progress step, the icon winds down with it.
                        onRunningChanged: syncButton.updateSpin()
                    }
                }
            }
        }

        // Sync button (roadmap D-6): the container is invisible but keeps the
        // hit area/proportions; the reload SVG is the whole control. While a
        // sync runs the container spins and the icon follows it.
        Rectangle {
            id: syncButton
            Layout.preferredWidth: root.height * 0.6
            Layout.preferredHeight: root.height * 0.6
            radius: Theme.radius
            color: "transparent"

            // Spin while the progress bar is moving (a sync runs / the fill is
            // still settling) and on hover (slow). The icon and the bar are
            // coupled: when the bar stops and the mouse is away, the spin
            // stops too. Restarting resumes from the current angle and
            // stopping leaves the icon in place, so nothing rewinds/bounces.
            function spinning() {
                return Backend.librarySyncing || progressAnim.running || syncHover.hovered;
            }

            function updateSpin() {
                if (spinning()) {
                    spin.stop();
                    spin.from = syncButton.rotation;
                    spin.to = spin.from + 360;
                    spin.duration = Backend.librarySyncing ? Theme.motion.spin : Theme.motion.spinSlow;
                    spin.start();
                } else {
                    spin.stop();
                    // A multiple of 360 is the same angle: bound the value
                    // without moving the icon.
                    syncButton.rotation = syncButton.rotation % 360;
                }
            }

            NumberAnimation {
                id: spin
                target: syncButton
                property: "rotation"
                loops: Animation.Infinite
                easing.type: Easing.Linear
            }

            Connections {
                target: Backend
                function onLibrary_syncing_changed() {
                    syncButton.updateSpin();
                }
            }

            VectorImage {
                anchors.centerIn: parent
                source: "qrc:/app/icons/reload-ui-2-svgrepo-com.svg"
                preferredRendererType: VectorImage.CurveRenderer
                width: root.height * 0.29
                height: width
                opacity: Backend.librarySyncing ? 0.38 : (syncHover.hovered ? 1.0 : 0.85)

                Behavior on opacity {
                    NumberAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                }
            }

            // Clickable affordance (reuse for every clickable): the hover
            // handler carries the pointing-hand cursor, the tap handler the
            // action. Pair them like this wherever something is clickable.
            HoverHandler {
                id: syncHover
                cursorShape: Qt.PointingHandCursor
                onHoveredChanged: syncButton.updateSpin()
            }
            TapHandler {
                enabled: !Backend.librarySyncing
                onTapped: Backend.syncLibrary()
            }
        }
    }
}
