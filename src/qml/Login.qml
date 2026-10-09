import QtQuick
import QtQuick.Controls
import musika

Item {
    Column {
        anchors.centerIn: parent
        width: 240
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
