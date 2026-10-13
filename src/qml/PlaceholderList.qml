import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Simple empty-state view used by libraries that are not indexed yet
// (Playlists, Radio).
Rectangle {
    id: root

    property string title: ""
    property string message: "Nothing here yet."

    color: Palette.surface

    // The view has no cursor; movement is a no-op.
    function handleAction(action) {
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Item {
            Layout.fillWidth: true
            implicitHeight: root.height * 0.1

            FadeText {
                anchors.verticalCenter: parent.verticalCenter
                anchors.left: parent.left
                anchors.leftMargin: root.width * 0.02
                text: root.title
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
            }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Column {
                anchors.centerIn: parent
                width: parent.width * 0.6
                spacing: root.height * 0.02

                FadeText {
                    width: parent.width
                    text: root.message
                    color: Palette.onSurfaceVariant
                    font.pixelSize: root.height * 0.024
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                }
                FadeText {
                    width: parent.width
                    text: "Not indexed locally yet."
                    color: Palette.outline
                    font.pixelSize: root.height * 0.018
                    horizontalAlignment: Text.AlignHCenter
                }
            }
        }
    }
}
