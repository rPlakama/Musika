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
    color: Theme.background
    title: qsTr("Musika")

    // Global key handler. It wraps the screen stack so unhandled keys bubble up
    // to it from whatever holds focus; focused controls (text fields) accept
    // their keys first, which keeps insert-mode typing working.
    Item {
        id: keyHandler
        anchors.fill: parent
        focus: true

        function dispatch(action) {
            // Space pauses/plays globally, regardless of screen or region.
            if (action === "play_pause") {
                Backend.playerToggle();
                return;
            }

            var current = stack.currentItem;
            if (current && current.handleAction)
                current.handleAction(action);
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
            initialItem: loginScreen

            Connections {
                target: Backend
                function onLogged_in_changed() {
                    if (Backend.loggedIn)
                        stack.replace(homeScreen);
                    else
                        stack.replace(loginScreen);
                }
            }
        }

        // Resolves deferred sequences (e.g. bare Space = play/pause).
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
            keyHandler.dispatch(command);
        }
    }

    Component.onCompleted: {
        Backend.startup();
        Backend.startIpc();
    }
}
