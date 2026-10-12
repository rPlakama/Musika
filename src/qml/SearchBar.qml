import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Theme.js" as Theme

// Placeholder for the global search entry (Space+F / Ctrl-F style).
// Static: no filtering happens yet.
Rectangle {
    id: root

    implicitHeight: parent ? parent.height * 0.085 : 0
    color: Theme.surface

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.02
        anchors.rightMargin: root.width * 0.02
        spacing: root.height * 0.15

        Text {
            text: "⌕"
            color: Theme.onSurfaceVariant
            font.pixelSize: root.height * 0.38
        }

        TextField {
            id: field
            Layout.fillWidth: true
            placeholderText: "Search artists, albums and songs…"
            color: Theme.onSurface
            placeholderTextColor: Theme.onSurfaceVariant
            font.pixelSize: root.height * 0.27
            background: Rectangle {
                color: Theme.surfaceContainerHigh
                radius: Theme.radius
                border.color: field.activeFocus ? Theme.primary : Theme.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }

        // Hint chip for the future global-search binding.
        Rectangle {
            implicitWidth: hint.implicitWidth + root.height * 0.3
            implicitHeight: root.height * 0.5
            radius: Theme.radius
            color: Theme.surfaceContainerHigh

            Text {
                id: hint
                anchors.centerIn: parent
                text: "Space F"
                color: Theme.onSurfaceVariant
                font.pixelSize: root.height * 0.22
            }
        }
    }
}
