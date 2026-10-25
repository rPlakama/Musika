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

// UI-59: the shared transport cluster. The previous / play-pause / next group
// is the only thing that contributes to this item's implicit size, so hosts
// keep that group centred. Repeat is an accessory drawn in the left gutter;
// toggling it never changes the transport's centre.
Item {
    id: root

    /// The reference size: skips are drawn at `unit` square, the play button
    /// scales up from it (a fraction of the bar/pane height at the call site).
    property real unit: 24
    /// Play-to-skip size ratio.
    readonly property real playRatio: 1.22
    property real spacing: root.unit * 0.11

    implicitWidth: core.implicitWidth
    implicitHeight: core.implicitHeight
    clip: false

    // The centring authority: previous / play / next only.
    RowLayout {
        id: core
        anchors.centerIn: parent
        spacing: root.spacing

        IconButton {
            id: previousButton
            Layout.preferredWidth: root.unit
            Layout.preferredHeight: root.unit
            Layout.minimumWidth: root.unit
            Layout.minimumHeight: root.unit
            Layout.alignment: Qt.AlignVCenter
            iconSource: "qrc:/app/icons/skip-previous-solid-svgrepo-com.svg"
            symbolRatio: 0.9
            onClicked: Backend.playerPrevious()
        }

        IconButton {
            id: playButton
            Layout.preferredWidth: root.unit * root.playRatio
            Layout.preferredHeight: root.unit * root.playRatio
            Layout.minimumWidth: root.unit * root.playRatio
            Layout.minimumHeight: root.unit * root.playRatio
            Layout.alignment: Qt.AlignVCenter
            symbolRatio: 0.6
            iconSource: Backend.playbackState === "playing"
                ? "qrc:/app/icons/pause-svgrepo-com.svg"
                : "qrc:/app/icons/play-1001-svgrepo-com.svg"
            symbolColor: Theme.iconStrong
            onClicked: Backend.playerToggle()
        }

        IconButton {
            id: nextButton
            Layout.preferredWidth: root.unit
            Layout.preferredHeight: root.unit
            Layout.minimumWidth: root.unit
            Layout.minimumHeight: root.unit
            Layout.alignment: Qt.AlignVCenter
            iconSource: "qrc:/app/icons/skip-next-circle-solid-svgrepo-com.svg"
            symbolRatio: 0.9
            onClicked: Backend.playerNext()
        }
    }

    // Accessory: sits in the left gutter, outside the centred core.
    IconButton {
        id: repeatButton
        anchors.verticalCenter: core.verticalCenter
        x: -width - root.spacing
        width: root.unit
        height: root.unit
        iconSource: "qrc:/app/icons/repeat-rounded.svg"
        symbolRatio: 0.9
        // Off keeps the slot but is visually absent; the click and Ctrl-R
        // still cycle it.
        opacity: Backend.repeatMode === 0 ? 0.0 : 1.0
        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.quick
                easing.type: Easing.Bezier
                easing.bezierCurve: Theme.ease.standard
            }
        }
        onClicked: Backend.cycleRepeat()

        Icon {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            width: parent.width * 0.38
            height: width
            source: "qrc:/app/icons/number-1.svg"
            symbolColor: Theme.iconStrong
            iconRatio: 0.78
            visible: Backend.repeatMode === 1
            z: 2
        }
    }

    // Press feedback: fires for keyboard, mouse and IPC alike.
    Connections {
        target: Backend
        function onRepeatModeChanged() {
            repeatButton.press();
        }
        function onTrackSkipped(forward) {
            if (forward)
                nextButton.press();
            else
                previousButton.press();
        }
        function onPlayToggled() {
            playButton.press();
        }
    }
}
