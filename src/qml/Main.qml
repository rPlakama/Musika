import QtQuick
import QtQuick.Controls
import musika

ApplicationWindow {
    visible: true
    width: 320
    height: 280
    title: qsTr("Musika")

    Item {
        id: main_login
        width: 240
        height: 210
        anchors.centerIn: parent

        Column {
            anchors.fill: parent
            spacing: 10

            TextField {
                id: serverURLField
                width: parent.width
                placeholderText: qsTr("Server URL")
                text: Backend.serverURL
                onEditingFinished: Backend.serverURL = text
            }
            TextField {
                id: usernameField
                width: parent.width
                placeholderText: qsTr("Username")
                text: Backend.username
                onEditingFinished: Backend.username = text
            }
            TextField {
                id: passwordField
                width: parent.width
                placeholderText: qsTr("Password")
                echoMode: TextInput.Password
                text: Backend.password
                onEditingFinished: Backend.password = text
            }

            Button {
                width: parent.width
                enabled: !Backend.busy
                text: Backend.busy ? qsTr("Logging in…") : qsTr("Log in")
                onClicked: Backend.login()
            }

            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                color: Backend.loggedIn ? "#2e7d32" : "#c62828"
                text: Backend.statusMessage
                visible: Backend.statusMessage.length > 0
            }
        }
    }
}
