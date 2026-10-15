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
import QtQuick.Controls
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// ARC-UI-2 M0: the `/` in-surface search command line — a one-line strip at
// the bottom edge, over whatever surface (or overlay) is on top, without a
// scrim. M0 is the input skeleton: open/close, typing, Esc abandons, Enter
// commits; the matcher, the `[i/N]` counter and the block highlights land in
// M1/M2 (SEARCHACTIONSTUDY.md §7).
Item {
    id: root

    property bool opened: false
    /// Distance from the window bottom: the player bar, or the fullscreen
    /// margin when the player view is up (Home owns both ratios).
    property real bottomOffset: 0

    function open() {
        opened = true;
        // The global keymap dies while the command line types (see
        // `Backend::key_event`).
        Backend.setLineSearchOpen(true);
        field.text = "";
        field.forceActiveFocus();
    }

    function close() {
        opened = false;
        Backend.setLineSearchOpen(false);
        Backend.releaseFocus();
    }

    /// Esc while typing abandons the search: cursor and session untouched.
    function abandon() {
        close();
    }

    /// Enter commits: M1 jumps the surface cursor and keeps the session;
    /// today the prompt just retreats.
    function commit() {
        close();
    }

    // Keymap actions that still reach Home while the prompt is up (IPC, or a
    // window key the field did not consume).
    function handleAction(action) {
        if (action === "enter")
            commit();
        else if (action === "back")
            abandon();
    }

    visible: opened || panel.opacity > 0
    z: Theme.zCommand

    Rectangle {
        id: panel
        width: root.width * 0.5
        height: root.height * 0.06
        x: (root.width - width) / 2
        // Rises from below the bar edge (motion §3.1) instead of scaling.
        y: root.height - height - root.bottomOffset + (root.opened ? 0 : height * 0.5)
        color: Palette.surface
        radius: Theme.radius
        border.color: Palette.primary
        border.width: Math.max(1, root.height * Theme.hairlineRatio)
        opacity: root.opened ? 1 : 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }
        Behavior on y {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }

        RowLayout {
            anchors.fill: parent
            anchors.margins: panel.height * 0.22
            spacing: panel.height * 0.18

            // The Neovim command-line prefix.
            Text {
                text: "/"
                color: Palette.primary
                font.family: Theme.fonts.body
                font.pixelSize: panel.height * 0.4
                font.bold: true
                Layout.alignment: Qt.AlignVCenter
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                TextField {
                    id: field
                    anchors.fill: parent
                    color: Palette.onSurface
                    font.family: Theme.fonts.body
                    font.pixelSize: panel.height * 0.32
                    background: Rectangle {
                        color: Palette.surfaceContainerHighest
                        radius: Theme.radius
                        border.color: field.activeFocus ? Palette.primary : Palette.outlineVariant
                        border.width: Math.max(1, root.height * Theme.hairlineRatio)
                    }

                    Keys.onPressed: event => {
                        // The keymap is dead while typing; consume the keys
                        // the field does not use itself so nothing bubbles to
                        // it (Tab would move focus, Ctrl-W would scope-cycle).
                        if (event.key === Qt.Key_Tab || event.key === Qt.Key_Backtab)
                            event.accepted = true;
                        else if (event.key === Qt.Key_W && (event.modifiers & Qt.ControlModifier))
                            event.accepted = true;
                    }
                    Keys.onReturnPressed: root.commit()
                    Keys.onEnterPressed: root.commit()
                    Keys.onEscapePressed: event => {
                        root.abandon();
                        event.accepted = true;
                    }
                    // History is M4; accept so the list cursor never moves.
                    Keys.onUpPressed: event => event.accepted = true
                    Keys.onDownPressed: event => event.accepted = true
                }

                // The hint mirrors the FTS bar's vanishing placeholder: it
                // stays while the command line is empty, INSERT included.
                FadeText {
                    anchors.fill: field
                    visible: field.text.length === 0
                    text: "Search this surface"
                    color: Palette.onSurfaceVariant
                    font.family: Theme.fonts.body
                    font.pixelSize: panel.height * 0.32
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    animated: false
                }
            }
        }
    }
}
