import QtQuick
import QtQuick.Layouts
import "Theme.js" as Theme

// "Mainscreen" mode entered from the inferior bar: album cover on the left,
// lyrics on the right. Static for now, no open/close wiring.
Rectangle {
    id: root

    color: Theme.background

    RowLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.03
        spacing: root.width * 0.03

        // Cover + track info.
        ColumnLayout {
            Layout.fillHeight: true
            Layout.preferredWidth: root.width * 0.4
            spacing: root.height * 0.02

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.maximumHeight: width
                Layout.alignment: Qt.AlignHCenter
                radius: Theme.radius
                color: Theme.primaryContainer

                Text {
                    anchors.centerIn: parent
                    text: "COVER"
                    color: Theme.onPrimaryContainer
                    font.pixelSize: root.height * 0.025
                    font.letterSpacing: root.height * 0.008
                }
            }

            Text {
                Layout.fillWidth: true
                text: "Weird Fishes / Arpeggi"
                color: Theme.onSurface
                font.pixelSize: root.height * 0.036
                font.bold: true
                elide: Text.ElideRight
            }
            Text {
                Layout.fillWidth: true
                text: "Radiohead  ·  In Rainbows"
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.022
                elide: Text.ElideRight
            }
        }

        // Lyrics.
        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: Theme.surfaceContainerLow

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: root.width * 0.015
                spacing: root.height * 0.01

                Text {
                    text: "Lyrics"
                    color: Theme.primary
                    font.pixelSize: root.height * 0.022
                    font.bold: true
                    font.letterSpacing: root.height * 0.004
                }

                Flickable {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    contentWidth: width
                    contentHeight: lyrics.implicitHeight
                    boundsBehavior: Flickable.StopAtBounds

                    Text {
                        id: lyrics
                        width: parent.width
                        text: "In the deepest ocean\nThe bottom of the sea\nYour eyes\nThey turn me\n\nWhy should I stay here?\nWhy should I stay?\n\nI'd be crazy not to follow\nFollow where you lead\nYour eyes\nThey turn me\n\nTurn me on to phantoms\nI follow to the edge of the earth\nAnd fall off\n\nEverybody leaves\nIf they get the chance\nAnd this is my chance"
                        color: Theme.onSurfaceVariant
                        font.pixelSize: root.height * 0.026
                        lineHeight: 1.4
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }

    // Affordance for returning to the library shell.
    Text {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.margins: root.width * 0.02
        text: "ESC  back to library"
        color: Theme.onSurfaceVariant
        font.pixelSize: root.width * 0.011
        font.letterSpacing: root.width * 0.001
    }
}
