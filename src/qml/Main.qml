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

    Component {
        id: loginScreen
        Login {}
    }

    Component {
        id: homeScreen
        Home {}
    }

    StackView {
        id: stack
        anchors.fill: parent
        initialItem: homeScreen

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

    // Global key handler. It owns focus when no text field is active, so vi
    // sequences reach the Rust engine; focused controls (e.g. the login fields)
    // still receive input directly.
    Item {
        id: keyHandler
        anchors.fill: parent
        focus: true

        Keys.onPressed: (event) => {
            var action = Backend.keyEvent(event.key, event.modifiers);
            if (action.length > 0) {
                event.accepted = true;
                var current = stack.currentItem;
                if (current && current.handleAction)
                    current.handleAction(action);
            }
        }
    }

    Component.onCompleted: Backend.startup()
}
