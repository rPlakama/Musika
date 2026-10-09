import QtQuick
import QtQuick.Controls
import musika

ApplicationWindow {
    visible: true
    width: 320
    height: 280
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
        initialItem: loginScreen

        Connections {
            target: Backend
            function onLogged_in_changed() {
                if (Backend.loggedIn)
                    stack.replace(homeScreen)
                else
                    stack.replace(loginScreen)
            }
        }
    }

    Component.onCompleted: Backend.startup()
}
