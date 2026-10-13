import QtQuick
import "Theme.js" as Theme

// Square icon button. Icons are Unicode glyphs so no icon font is required.
// Size comes from the layout (or the parent); the glyph is a fraction of it.
//
// State layers (R6): hover 8%, focus 10% primary + hairline, press 10%,
// disabled 12%/38%. Never colour alone.
Rectangle {
    id: root

    property string symbol: "▶"
    property real symbolRatio: 0.5
    property bool accent: false
    property alias hovered: hover.hovered

    signal clicked()

    radius: Theme.radius
    color: accent ? Theme.onPrimaryContainer : "transparent"
    border.width: activeFocus ? Math.max(1, width * Theme.hairlineRatio) : 0
    border.color: Theme.primary

    Rectangle {
        anchors.fill: parent
        radius: root.radius
        color: {
            if (!root.enabled)
                return Theme.stateDisabledContainer;
            if (tap.pressed)
                return Theme.statePress;
            if (root.activeFocus)
                return Theme.stateFocus;
            if (hover.hovered)
                return root.accent ? Theme.stateFocus : Theme.stateHover;
            return "transparent";
        }

        Behavior on color {
            ColorAnimation { duration: Theme.motion.micro; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
        }
    }

    FadeText {
        anchors.centerIn: parent
        text: root.symbol
        font.pixelSize: root.width * root.symbolRatio
        color: !root.enabled
            ? Theme.stateDisabledContent
            : (root.accent ? Theme.onPrimary : Theme.onSurface)
        animated: false
    }

    HoverHandler {
        id: hover
    }

    TapHandler {
        id: tap
        onTapped: root.clicked()
    }
}
