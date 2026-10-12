import QtQuick
import QtQuick.Layouts
import "Theme.js" as Theme

// Left sidebar: the library segments (Albums / Songs / Artists / Playlists /
// Radio), a brand header and the library sync entry point.
// Static: the selection does not move yet.
Rectangle {
    id: root

    color: Theme.surfaceContainerLow

    Rectangle {
        id: brand
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: root.height * 0.09
        color: "transparent"

        Text {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.leftMargin: root.width * 0.1
            text: "MUSIKA"
            color: Theme.primary
            font.pixelSize: root.height * 0.03
            font.bold: true
            font.letterSpacing: root.height * 0.005
        }
    }

    ColumnLayout {
        anchors.top: brand.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: root.width * 0.07
        spacing: root.height * 0.004

        Repeater {
            model: ["Albums", "Songs", "Artists", "Playlists", "Radio"]

            delegate: Rectangle {
                id: segment
                required property int index
                required property string modelData

                Layout.fillWidth: true
                implicitHeight: root.height * 0.064
                radius: Theme.radius
                color: index === 0
                    ? Theme.secondaryContainer
                    : (segmentHover.hovered ? Theme.surfaceContainerHigh : "transparent")

                // Accent bar on the active segment.
                Rectangle {
                    anchors.left: parent.left
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.width * 0.02
                    color: Theme.primary
                    visible: segment.index === 0
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: root.width * 0.1
                    text: segment.modelData
                    color: segment.index === 0 ? Theme.onSecondaryContainer : Theme.onSurfaceVariant
                    font.pixelSize: root.height * 0.022
                    font.bold: segment.index === 0
                }

                HoverHandler {
                    id: segmentHover
                }
            }
        }
    }

    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: syncButton.top
        anchors.margins: root.width * 0.1
        height: Math.max(1, root.height * Theme.hairlineRatio)
        color: Theme.outlineVariant
    }

    // TODO(movementation): the status bar drives syncing; this is a local entry.
    Rectangle {
        id: syncButton
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: root.width * 0.1
        height: root.height * 0.058
        radius: Theme.radius
        color: syncHover.hovered ? Theme.surfaceContainerHighest : "transparent"
        border.color: Theme.outlineVariant
        border.width: Math.max(1, root.width * Theme.hairlineRatio)

        Text {
            anchors.centerIn: parent
            text: "⟳  Sync library"
            color: Theme.onSurfaceVariant
            font.pixelSize: root.height * 0.021
        }

        HoverHandler {
            id: syncHover
        }
    }
}
