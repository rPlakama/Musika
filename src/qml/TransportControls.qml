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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// UI-59: the one transport cluster — previous / play-pause / next on a single
// horizontal line, identical in the player bar and the fullscreen transport.
// The buttons are unsqueezable (`Layout.minimum*` == preferred) and share a
// common centre line, so a tight layout clips the surrounding zones first
// instead of deforming the cluster; the play button is proportionally larger
// and sits at the row's centre. The skip/play press feedback lives here, so
// hosts never duplicate it (CRITIQUEQML QA-5).
RowLayout {
    id: root

    /// The reference size: skips are drawn at `unit` square, the play button
    /// scales up from it (a fraction of the bar/pane height at the call site).
    property real unit: 24
    /// Play-to-skip size ratio.
    readonly property real playRatio: 1.22

    spacing: root.unit * 0.11

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

    // Press feedback: fires for keyboard, mouse and IPC alike.
    Connections {
        target: Backend
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
