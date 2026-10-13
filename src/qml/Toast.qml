import QtQuick
import QtQuick.Layouts
import "Theme.js" as Theme

// Transient error/notification banner.
Rectangle {
    id: root

    property string message: ""
    property string kind: ""
    property bool shown: false
    // Motion: quick in, lazy out (nothing keyboard-critical blocks on it).
    property real fadeDuration: Theme.motion.quick

    readonly property bool syncing: kind === "sync"

    signal retry()

    color: syncing ? Theme.tertiaryContainer : Theme.errorContainer
    radius: Theme.radius
    border.width: Math.max(1, height * Theme.hairlineRatio)
    border.color: syncing ? Theme.tertiary : Theme.error
    opacity: shown ? 1 : 0
    visible: opacity > 0

    Behavior on opacity {
        NumberAnimation { duration: root.fadeDuration; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
    }

    function show(kind, message) {
        root.kind = kind;
        root.message = message;
        root.fadeDuration = Theme.motion.quick;
        root.shown = true;
        hideTimer.restart();
    }

    Timer {
        id: hideTimer
        interval: 6000
        onTriggered: {
            root.fadeDuration = Theme.motion.lazy;
            root.shown = false;
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.03
        anchors.rightMargin: root.width * 0.03
        spacing: root.width * 0.03

        FadeText {
            text: "⚠"
            color: root.syncing ? Theme.onTertiaryContainer : Theme.onErrorContainer
            font.pixelSize: Theme.clamp(root.height * 0.36, 12, 18)
            animated: false
        }

        FadeText {
            Layout.fillWidth: true
            text: root.message
            color: root.syncing ? Theme.onTertiaryContainer : Theme.onErrorContainer
            font.pixelSize: Theme.clamp(root.height * 0.3, 12, 18)
            elide: Text.ElideRight
            animated: false
        }

        Rectangle {
            visible: root.syncing
            implicitWidth: retryLabel.implicitWidth + root.height * 0.5
            implicitHeight: root.height * 0.62
            radius: Theme.radius
            color: retryHover.hovered ? Theme.primary : Theme.primaryContainer

            FadeText {
                id: retryLabel
                anchors.centerIn: parent
                text: "Retry"
                color: Theme.onPrimaryContainer
                font.pixelSize: Theme.clamp(root.height * 0.28, 10, 14)
                animated: false
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
