import QtQuick
import QtQuick.Controls
import musika
import "Theme.js" as Theme

Item {
    id: root

    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    Column {
        anchors.centerIn: parent
        width: root.width * 0.224
        spacing: root.height * 0.014

        Text {
            width: parent.width
            text: "MUSIKA"
            color: Theme.primary
            font.pixelSize: root.height * 0.035
            font.bold: true
            font.letterSpacing: root.height * 0.0084
            horizontalAlignment: Text.AlignHCenter
        }

        TextField {
            id: serverURLField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Server URL")
            placeholderTextColor: Theme.onSurfaceVariant
            color: Theme.onSurface
            font.pixelSize: root.height * 0.0175
            text: Backend.serverURL
            onEditingFinished: Backend.serverURL = text
            background: Rectangle {
                color: Theme.surfaceContainerHigh
                radius: Theme.radius
                border.color: serverURLField.activeFocus ? Theme.primary : Theme.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }
        TextField {
            id: usernameField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Username")
            placeholderTextColor: Theme.onSurfaceVariant
            color: Theme.onSurface
            font.pixelSize: root.height * 0.0175
            text: Backend.username
            onEditingFinished: Backend.username = text
            background: Rectangle {
                color: Theme.surfaceContainerHigh
                radius: Theme.radius
                border.color: usernameField.activeFocus ? Theme.primary : Theme.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }
        TextField {
            id: passwordField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Password")
            placeholderTextColor: Theme.onSurfaceVariant
            color: Theme.onSurface
            font.pixelSize: root.height * 0.0175
            echoMode: TextInput.Password
            text: Backend.password
            onEditingFinished: Backend.password = text
            background: Rectangle {
                color: Theme.surfaceContainerHigh
                radius: Theme.radius
                border.color: passwordField.activeFocus ? Theme.primary : Theme.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }

        Button {
            id: loginButton
            width: parent.width
            implicitHeight: root.height * 0.049
            enabled: !Backend.busy
            text: Backend.busy ? qsTr("Logging in…") : qsTr("Log in")
            onClicked: Backend.login()
            background: Rectangle {
                radius: Theme.radius
                color: !loginButton.enabled
                    ? Theme.surfaceContainerHigh
                    : (loginButton.hovered ? Theme.primary : Theme.primaryContainer)
            }
            contentItem: Text {
                text: loginButton.text
                color: Theme.onPrimaryContainer
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                font.pixelSize: root.height * 0.0175
            }
        }

        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Backend.loggedIn ? Theme.primary : Theme.error
            text: Backend.statusMessage
            visible: Backend.statusMessage.length > 0
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: root.height * 0.014
        }
    }
}
