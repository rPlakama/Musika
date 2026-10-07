import QtQuick
import QtQuick.Controls
import musika

ApplicationWindow {

    visible: true
    title: qsTr("Minimal QML app")

    Button {
        anchors.centerIn: parent
        text: "Ping Server"
        onClicked: Backend.say_hello()
    }
}
