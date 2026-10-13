import QtQuick
import "Theme.js" as Theme

// Text that fades in when it is created. Drop-in replacement for `Text`.
// Set `animated: false` for content that is re-created while scrolling
// (list/search delegates) so it does not blink.
Text {
    id: root

    property bool animated: true

    NumberAnimation on opacity {
        running: root.animated
        from: 0
        to: 1
        duration: Theme.motion.quick
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
    }
}
