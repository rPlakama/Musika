import QtQuick
import "Theme.js" as Theme

// Text that fades in when it is created. Drop-in replacement for `Text`.
Text {
    NumberAnimation on opacity {
        from: 0
        to: 1
        duration: Theme.duration
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.easeOut
    }
}
