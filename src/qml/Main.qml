import QtQuick
import QtQuick.Controls
import musika

ApplicationWindow {

    visible: true
    title: qsTr("Musika")

    Item {
        id: main_login
        width: 200
        height: 100
        anchors.centerIn: parent

        TextField {
            id: usernameField
            placeholderText: qsTr("Enter Username")

            anchors {
                top: parent.top
                horizontalCenter: parent.horizontalCenter
            }
        }

        TextField {
            id: passwordField
            placeholderText: qsTr("Enter Password")
            anchors {
                bottom: parent.bottom
                horizontalCenter: parent.horizontalCenter
            }
        }
    }
}
