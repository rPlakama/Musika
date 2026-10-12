import QtQuick
import "Theme.js" as Theme

// Square icon button. Icons are Unicode glyphs so no icon font is required.
// Size comes from the layout (or the parent); the glyph is a fraction of it.
// Static for now: no actions are wired while the UI is being laid out.
Rectangle {
    id: root

    property string symbol: "▶"
    property real symbolRatio: 0.5
    property bool accent: false
    property alias hovered: hover.hovered

    signal clicked()

    radius: Theme.radius
    color: accent
        ? (hover.hovered ? Theme.primary : Theme.onPrimaryContainer)
        : (hover.hovered ? Theme.surfaceContainerHighest : "transparent")

    Text {
        anchors.centerIn: parent
        text: root.symbol
        font.pixelSize: root.width * root.symbolRatio
        color: root.accent ? Theme.onPrimary : Theme.onSurface
    }

    HoverHandler {
        id: hover
    }

    TapHandler {
        onTapped: root.clicked()
    }
}
