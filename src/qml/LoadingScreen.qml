import QtQuick
import musika
import "Theme.js" as Theme

// Startup screen shown when cached credentials exist: pings the server with a
// visible 5s countdown before falling back to the login screen.
Item {
    id: root

    signal timedOut()

    property int timeoutMs: 5000
    property int remaining: timeoutMs

    Rectangle {
        anchors.fill: parent
        color: Palette.background
    }

    Column {
        anchors.centerIn: parent
        width: root.width * 0.6
        spacing: root.height * 0.02

        FadeText {
            width: parent.width
            text: "Waiting for Server-ping"
            color: Palette.onSurface
            font.pixelSize: root.height * 0.05
            horizontalAlignment: Text.AlignHCenter
        }

        FadeText {
            width: parent.width
            text: "(timeout " + root.remaining + "ms)"
            color: Palette.onSurfaceVariant
            font.pixelSize: root.height * 0.03
            horizontalAlignment: Text.AlignHCenter
        }

        Rectangle {
            width: parent.width
            height: Math.max(2, root.height * 0.006)
            color: Palette.outlineVariant

            Rectangle {
                width: parent.width * (1 - root.remaining / root.timeoutMs)
                height: parent.height
                color: Palette.primary
            }
        }
    }

    Timer {
        interval: 100
        running: root.visible
        repeat: true
        onTriggered: {
            root.remaining -= 100;
            if (root.remaining <= 0) {
                stop();
                root.timedOut();
            }
        }
    }
}
