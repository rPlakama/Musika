import QtQuick
import QtQuick.Controls
import musika

ApplicationWindow {
    visible: true
    title: qsTr("Musika")

    Item {
        id: main_login
        width: 200
        height: 150
        anchors.centerIn: parent

        Column {
            anchors.fill: parent
            spacing: 10

            TextField {
                id: serverURLField
                text: backend.serverUrl
                onEditingFinished: backend.serverUrl = serverURLField.text
            }
            TextField {
                id: usernameField
                text: backend.username
                onEditingFinished: backend.username = usernameField.text
            }

            TextField {
                id: passwordField
                text: backend.password
                onEditingFinished: backend.password = passwordField.text
            }

            button {
                text: "Log in"
                onClicked: backend.login()
            }
        }
    }
}
