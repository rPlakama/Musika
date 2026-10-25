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
    id: root
    visible: true
    width: Screen.width * 0.68
    height: Screen.height * 0.72
    minimumWidth: 0
    minimumHeight: Screen.height * 0.4
    color: Palette.background
    title: qsTr("Musika")

    // Debug only: report the first paint, then stay quiet.
    property bool frontendReadySent: false
    onAfterRendering: {
        if (!frontendReadySent) {
            frontendReadySent = true;
            Backend.frontendReady();
        }
    }

    // P-6: the session debounce cannot be awaited during teardown, so the
    // backend does one synchronous last-chance write (own DB, single row).
    Loader {
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.margins: 8
        z: 1000
        active: Backend.debugBuild()
        source: "FpsCounter.qml"
    }

    onClosing: Backend.flushStatus()

    // ARC-43: a focus loss can swallow the Alt release; never leave the
    // quick-action overlay armed across applications.
    onActiveChanged: if (!active) Backend.setAltHeld(false)

    // UI-40: `Palette.apply` snaps every role, so a cover swap is mixed here
    // (the colors payload arrives here) and pushed back through `apply` each
    // animation step. The first apply, resets and `reduceMotion` snap.
    property bool paletteAnimated: false
    property var paletteFrom: ({})
    property var paletteTo: ({})
    property real paletteT: 1

    // Payload roles are snake_case, QML roles camelCase (`inverse_on_surface`
    // is the one key that maps around a `Palette` property name).
    function paletteCamel(role) {
        if (role === "inverse_on_surface")
            return "onInverseSurface";
        return role.replace(/_([a-z])/g, function (_, c) { return c.toUpperCase() });
    }

    function pushPalette(t) {
        var mixed = {};
        for (var role in paletteTo) {
            var to = Qt.color(paletteTo[role]);
            var from = paletteFrom[role] !== undefined ? Qt.color(paletteFrom[role]) : to;
            mixed[role] = Qt.rgba(
                from.r + (to.r - from.r) * t,
                from.g + (to.g - from.g) * t,
                from.b + (to.b - from.b) * t,
                from.a + (to.a - from.a) * t).toString();
        }
        Palette.apply(JSON.stringify({ dark: mixed }));
    }

    function fadePalette(dark) {
        if (!paletteAnimated || Theme.reduceMotion) {
            paletteAnimated = true;
            paletteFade.stop();
            Palette.apply(JSON.stringify({ dark: dark }));
            return;
        }
        var from = {};
        // The painted colors, so a re-target mid-fade continues from them.
        for (var role in dark)
            from[role] = Palette[paletteCamel(role)];
        paletteFrom = from;
        paletteTo = dark;
        paletteFade.restart();
    }

    onPaletteTChanged: pushPalette(paletteT)

    NumberAnimation {
        id: paletteFade
        target: root
        property: "paletteT"
        from: 0
        to: 1
        duration: Theme.motion.standard
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
    }

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
                Backend.playerAdjustVolume(5 * Math.max(1, Backend.keyCount));
                return;
            }
            if (action === "volume_down") {
                Backend.playerAdjustVolume(-5 * Math.max(1, Backend.keyCount));
                return;
            }

            // Clipboard verbs (VI-21): `y` yanks the playing track to the
            // system clipboard. VI-8a: any focused row surface (or the queue
            // drawer) takes `y`/`Y` for the universal row clipboard; the
            // text copy is the fallback only when nothing can be yanked.
            if (action === "yank") {
                var yankTarget = stack.currentItem;
                if (yankTarget && yankTarget["canYankRows"] === true) {
                    yankTarget.handleAction(action);
                    return;
                }
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
                // Ctrl-V stays the system text paste (VI-21); the queue row
                // clipboard rides `p`/`P` (VI-8a).
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
            // ARC-43: bare Alt opens the quick-action menu; it must not feed
            // the keymap, so a half-typed sequence stays pending (which keeps
            // the overlay disarmed).
            if (event.key === Qt.Key_Alt) {
                Backend.setAltHeld(true);
                return;
            }
            // ARC-43: with Alt held, h/j/k/l (or the arrow keys) aim the
            // quick-action quadrants — releasing Alt fires the aim, releasing
            // in the neutral center fires nothing — and Esc cancels. These
            // keys never reach the keymap.
            if (Backend.altHeld) {
                // UI-60: Shift-J/K step the aimed favourite slot's rating.
                // They fire once per press (key auto-repeat would flood the
                // server); a non-rating aim simply ignores them.
                if (!event.isAutoRepeat
                        && (event.modifiers & Qt.ShiftModifier)
                        && (event.key === Qt.Key_J || event.key === Qt.Key_K)) {
                    dispatch("quick_rating "
                        + (event.key === Qt.Key_K ? "up" : "down"));
                    return;
                }
                var aim = "";
                if (event.key === Qt.Key_H || event.key === Qt.Key_Left)
                    aim = "west";
                else if (event.key === Qt.Key_J || event.key === Qt.Key_Down)
                    aim = "south";
                else if (event.key === Qt.Key_K || event.key === Qt.Key_Up)
                    aim = "north";
                else if (event.key === Qt.Key_L || event.key === Qt.Key_Right)
                    aim = "east";
                if (aim.length > 0) {
                    dispatch("quick_aim " + aim);
                    return;
                }
                // Esc is not the Alt gesture's cancel: releasing over the
                // neutral centre is. While Alt is held, Esc is swallowed so
                // it can neither cancel the menu nor leak into the keymap.
                if (event.key === Qt.Key_Escape)
                    return;
            }
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

        Keys.onReleased: (event) => {
            if (event.key === Qt.Key_Alt)
                Backend.setAltHeld(false);
        }

        StackView {
            id: stack
            anchors.fill: parent
            initialItem: loginScreen

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
            }
        }

        // ARC-45: the mouse Back/Forward buttons walk the travel log. Only
        // those two buttons are accepted, so clicks and wheel stay untouched.
        TapHandler {
            acceptedButtons: Qt.BackButton | Qt.ForwardButton
            onTapped: (eventPoint, button) => {
                if (button === Qt.BackButton)
                    keyHandler.dispatch("prev_place");
                else if (button === Qt.ForwardButton)
                    keyHandler.dispatch("next_place");
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

    // Dynamic palette (THM-3/THM-4 + UI-40): generate once per playing cover,
    // apply the payload only while it still matches the current art, and
    // crossfade it in.
    Connections {
        target: Backend
        function onCurrent_cover_art_changed() {
            if (Backend.currentCoverArt.length > 0) {
                Backend.requestColors();
            } else {
                paletteFade.stop();
                Palette.reset();
            }
        }
        function onDataReady(channel, payload) {
            if (channel !== "colors")
                return;
            var data = JSON.parse(payload);
            if (data.cover_art !== Backend.currentCoverArt)
                return;
            if (!data.dark) {
                paletteFade.stop();
                Palette.reset();
                return;
            }
            root.fadePalette(data.dark);
        }
    }

    Component.onCompleted: {
        Backend.startup();
        Backend.startIpc();
    }
}
