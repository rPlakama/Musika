import QtQuick
import QtQuick.Layouts
import "Theme.js" as Theme

// Transient error/notification banner.
Rectangle {
    id: root

    property string message: ""
    property string kind: ""
    property bool shown: false

    signal retry()

    color: Theme.surfaceContainerHighest
    radius: Theme.radius
    border.width: Math.max(1, height * 0.03)
    border.color: kind === "sync" ? Theme.tertiary : Theme.error
    opacity: shown ? 1 : 0
    visible: opacity > 0

    Behavior on opacity {
        NumberAnimation { duration: Theme.duration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.easeOut }
    }

    function show(kind, message) {
        root.kind = kind;
        root.message = message;
        root.shown = true;
        hideTimer.restart();
    }

    Timer {
        id: hideTimer
        interval: 6000
        onTriggered: root.shown = false
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.03
        anchors.rightMargin: root.width * 0.03
        spacing: root.width * 0.03

        FadeText {
            text: "⚠"
            color: root.kind === "sync" ? Theme.tertiary : Theme.error
            font.pixelSize: root.height * 0.36
        }

        FadeText {
            Layout.fillWidth: true
            text: root.message
            color: Theme.onSurface
            font.pixelSize: root.height * 0.3
            elide: Text.ElideRight
        }

        Rectangle {
            visible: root.kind === "sync"
            implicitWidth: retryLabel.implicitWidth + root.height * 0.5
            implicitHeight: root.height * 0.62
            radius: Theme.radius
            color: retryHover.hovered ? Theme.primary : Theme.primaryContainer

            FadeText {
                id: retryLabel
                anchors.centerIn: parent
                text: "Retry"
                color: Theme.onPrimaryContainer
                font.pixelSize: root.height * 0.28
            }

            HoverHandler {
                id: retryHover
            }
            TapHandler {
                onTapped: {
                    root.shown = false;
                    root.retry();
                }
            }
        }
    }
}
