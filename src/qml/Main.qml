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
import QtQuick.Window
import musika
import "Theme.js" as Theme

ApplicationWindow {
    visible: true
    width: Screen.width * 0.68
    height: Screen.height * 0.72
    minimumWidth: Screen.width * 0.4
    minimumHeight: Screen.height * 0.4
    color: Palette.background
    title: qsTr("Musika")

    // Global key handler. It wraps the screen stack so unhandled keys bubble up
    // to it from whatever holds focus; focused controls (text fields) accept
    // their keys first, which keeps insert-mode typing working.
    Item {
        id: keyHandler
        anchors.fill: parent
        focus: true

        function dispatch(action) {
            // Mute and volume steps are global like Space (media-key style).
            if (action === "mute") {
                Backend.playerToggleMute();
                return;
            }
            if (action === "volume_up") {
                Backend.playerAdjustVolume(5);
                return;
            }
            if (action === "volume_down") {
                Backend.playerAdjustVolume(-5);
                return;
            }

            // Clipboard verbs (VI-21): `y` yanks the playing track to the
            // system clipboard, `P` pastes a track reference back into
            // playback.
            if (action === "yank") {
                var yanked = Backend.currentArtist.length > 0
                    ? Backend.currentArtist + " - " + Backend.currentTitle
                    : Backend.currentTitle;
                if (yanked.length > 0) {
                    clipboard.text = yanked;
                    clipboard.selectAll();
                    clipboard.copy();
                }
                return;
            }
            if (action === "paste") {
                clipboard.text = "";
                clipboard.selectAll();
                clipboard.paste();
                var target = stack.currentItem;
                if (clipboard.text.length > 0 && target && target["pasteSearch"])
                    target["pasteSearch"](clipboard.text);
                return;
            }

            // Shift+Enter toggles the fullscreen player globally; the screen
            // that owns the state implements `toggleFullscreen`.
            if (action === "toggle_fullscreen") {
                var screen = stack.currentItem;
                if (screen && screen.toggleFullscreen)
                    screen.toggleFullscreen();
                return;
            }

            var current = stack.currentItem;
            if (current && current.handleAction)
                current.handleAction(action);
        }

        // Native MPRIS transport (Q-8); payload commands carry their value
        // after a colon. Global, unlike screen- and context-specific actions.
        function mprisDispatch(command) {
            if (command === "mpris_play") {
                if (Backend.playbackState !== "playing")
                    Backend.playerToggle();
            } else if (command === "mpris_pause") {
                if (Backend.playbackState === "playing")
                    Backend.playerToggle();
            } else if (command === "mpris_toggle") {
                Backend.playerToggle();
            } else if (command === "mpris_stop") {
                Backend.playerStop();
            } else if (command === "mpris_next") {
                Backend.playerNext();
            } else if (command === "mpris_prev") {
                Backend.playerPrevious();
            } else if (command.indexOf("mpris_seek:") === 0) {
                Backend.playerSeek(Math.max(0,
                    Backend.position + Number(command.slice(11)) / 1000000));
            } else if (command.indexOf("mpris_position:") === 0) {
                Backend.playerSeek(Number(command.slice(15)) / 1000000);
            } else if (command.indexOf("mpris_volume:") === 0) {
                Backend.playerSetVolume(Number(command.slice(13)) * 100);
            }
        }

        // System clipboard bridge (VI-21): the hidden TextEdit is the only
        // QML-side clipboard accessor; yank writes, paste reads.
        TextEdit {
            id: clipboard
            width: 0
            height: 0
            visible: false
        }

        Keys.onPressed: (event) => {
            var action = Backend.keyEvent(event.key, event.modifiers);
            if (action === "PENDING") {
                keyTimer.restart();
                return;
            }
            keyTimer.stop();
            if (action.length > 0) {
                event.accepted = true;
                dispatch(action);
            }
        }

        StackView {
            id: stack
            anchors.fill: parent
            initialItem: loadingScreen

            Connections {
                target: Backend
                function onLogged_in_changed() {
                    if (Backend.loggedIn)
                        stack.replace(homeScreen);
                    else
                        stack.replace(loginScreen);
                }
                function onLoginRequired() {
                    stack.replace(loginScreen);
                }
                function onLoginPending() {
                    stack.replace(loadingScreen);
                }
            }
        }

        // Resolves deferred sequences once the inter-key timeout elapses
        // (e.g. `g g`, `shift-space f`); bare Space fires immediately.
        Timer {
            id: keyTimer
            interval: 800
            onTriggered: {
                var action = Backend.keyTimeout();
                if (action.length > 0)
                    keyHandler.dispatch(action);
            }
        }
    }

    Component {
        id: loginScreen
        Login {}
    }

    Component {
        id: homeScreen
        Home {}
    }

    Component {
        id: loadingScreen
        LoadingScreen {
            onTimedOut: if (!Backend.loggedIn)
                stack.replace(loginScreen)
        }
    }

    // Regain keyboard focus (e.g. after leaving the search field).
    Connections {
        target: Backend
        function onFocusMain() {
            keyHandler.forceActiveFocus();
        }
    }

    // Commands from the external IPC socket run through the same dispatcher.
    Connections {
        target: Backend
        function onCommandIssued(command) {
            // Manual palette refresh (see THM-5); everything else is a key
            // action name.
            if (command === "colors_refresh") {
                Backend.refreshColors();
                return;
            }
            if (command === "lyrics_refresh") {
                Backend.refreshLyrics();
                return;
            }
            // MPRIS commands (Q-8) use the same pump: they may call back into
            // the backend, which is still borrowed during this signal.
            if (command.indexOf("mpris_") === 0) {
                Qt.callLater(() => keyHandler.mprisDispatch(command));
                return;
            }
            // Pump, don't re-enter: a command may call back into the backend,
            // which is still borrowed while this signal is being emitted.
            Qt.callLater(() => keyHandler.dispatch(command));
        }
    }

    // Dynamic palette (THM-3/THM-4): generate once per playing cover, apply
    // the payload only while it still matches the current art.
    Connections {
        target: Backend
        function onCurrent_cover_art_changed() {
            if (Backend.currentCoverArt.length > 0)
                Backend.requestColors();
            else
                Palette.reset();
        }
        function onDataReady(channel, payload) {
            if (channel !== "colors")
                return;
            if (JSON.parse(payload).cover_art === Backend.currentCoverArt)
                Palette.apply(payload);
        }
    }

    Component.onCompleted: {
        Backend.startup();
        Backend.startIpc();
    }
}
