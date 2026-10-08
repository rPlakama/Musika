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
                placeholderText: qsTr("Enter ServerURL")
                anchors.horizontalCenter: parent.horizontalCenter
            }

            TextField {
                id: usernameField
                placeholderText: qsTr("Enter Username")
                anchors.horizontalCenter: parent.horizontalCenter
            }

            TextField {
                id: passwordField
                placeholderText: qsTr("Enter Password")
                anchors.horizontalCenter: parent.horizontalCenter
            }
        }
    }
}
