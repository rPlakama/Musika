// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

import QtQuick
import musika
import "Theme.js" as Theme

// ARC-43: Alt-hold quick actions, keyboard gesture. Holding Alt overlays
// four quadrants around the current item; `h`/`j`/`k`/`l` (or the arrow keys)
// aim west/south/north/east and releasing Alt is Enter (it fires the aimed
// action). The center is the neutral aim: releasing there fires nothing, Esc
// dismisses for the rest of the hold, and aiming toward the opposite quadrant
// first passes through neutral (north + `j` = neutral, `j` again = south).
// The host arms it with the current item and a `{north,south,east,west}`
// config of `{label, invoke}` slots (null = open, drawn as "—"), so the
// contextual Go to Album / Go to Artist swap is config, not branches. The
// option tiles are acrylic panels (blurred backdrop, like the search
// overlay).
Item {
    id: root

    property bool armed: false
    property Item targetItem: null
    property var config: null
    property string aimed: ""
    /// UI-55: the item behind the option tiles whose pixels feed the acrylic
    /// blur (the shell, or the fullscreen player when it is up).
    property Item backdrop: null
    /// Esc retired this hold; arming stays off until Alt is released.
    property bool dismissed: false

    readonly property bool available: !Backend.searchOpen && !Backend.keyPending

    readonly property point anchor: (targetItem && targetItem.visible && targetItem.width > 0)
        ? targetItem.mapToItem(root, targetItem.width / 2, targetItem.height / 2)
        : Qt.point(-1000, -1000)
    readonly property real gap: Math.max(10, height * 0.02)
    /// Option tiles run 70% larger than the first pass (ARC-43 review).
    readonly property real optionScale: 1.7
    readonly property real boxFont: Math.max(11, height * 0.018) * optionScale
    readonly property real boxPadding: Math.max(6, height * 0.008) * optionScale
    readonly property real targetWidth: targetItem ? targetItem.width : 0
    readonly property real targetHeight: targetItem ? targetItem.height : 0

    opacity: armed ? 1 : 0
    visible: opacity > 0

    Behavior on opacity {
        NumberAnimation {
            duration: Theme.motion.quick
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.standard
        }
    }

    function arm(item, cfg) {
        if (!available || dismissed || !item || !cfg)
            return;
        targetItem = item;
        config = cfg;
        aimed = "";
        armed = true;
    }

    function cancel() {
        armed = false;
        aimed = "";
    }

    /// Esc: cancel and keep the overlay down for the rest of this Alt hold.
    function dismiss() {
        dismissed = true;
        cancel();
    }

    function opposite(direction) {
        if (direction === "north")
            return "south";
        if (direction === "south")
            return "north";
        if (direction === "east")
            return "west";
        return "east";
    }

    /// Keyboard aiming (h/j/k/l, arrows). The center is a real neutral state:
    /// aiming toward the quadrant opposite the current one steps back to
    /// neutral first, the next press continues to that side. An open slot is
    /// never aimed — a press toward it lands on neutral.
    function aim(direction) {
        if (!armed)
            return;
        var next = aimed === opposite(direction) ? "" : direction;
        if (next.length > 0 && (!config || !config[next]))
            next = "";
        aimed = next;
    }

    function confirm() {
        var slot = aimed.length > 0 && config ? config[aimed] : null;
        if (slot && slot.invoke)
            slot.invoke();
        cancel();
    }

    // Alt release confirms; a modal context or a half-typed sequence retires
    // the overlay instead (the roadmap hard rules).
    Connections {
        target: Backend
        function onAlt_held_changed() {
            if (!Backend.altHeld) {
                root.dismissed = false;
                root.confirm();
            }
        }
        function onKey_pending_changed() {
            if (Backend.keyPending)
                root.cancel();
        }
        function onSearch_open_changed() {
            if (Backend.searchOpen)
                root.cancel();
        }
    }

    // While armed, swallow clicks so an errant press cannot activate the row
    // underneath mid-gesture; hover passes through (no hoverEnabled).
    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        enabled: root.armed
    }

    // Center mark: the neutral aim. Grows while neutral so the state reads.
    Rectangle {
        readonly property point at: root.anchor
        visible: root.armed
        x: at.x - width / 2
        y: at.y - height / 2
        width: Math.max(5, root.height * 0.008) * (root.aimed === "" ? 1.6 : 1.0)
        height: width
        radius: width / 2
        color: Palette.primary

        Behavior on width {
            NumberAnimation { duration: Theme.motion.micro }
        }
    }

    Repeater {
        model: ["north", "south", "east", "west"]

        delegate: AcrylicPanel {
            id: box
            required property string modelData

            readonly property var slot: root.config ? root.config[box.modelData] : null
            readonly property bool isAimed: root.aimed === box.modelData

            width: Math.max(boxLabel.implicitWidth + root.boxPadding * 2,
                            root.height * 0.09 * root.optionScale)
            height: boxLabel.implicitHeight + root.boxPadding * 2
            radius: 0
            backdrop: root.backdrop
            tint: box.isAimed ? Palette.primaryContainer : Palette.surfaceContainerHigh
            border.width: 1
            border.color: box.isAimed ? Palette.primary : Palette.outline
            opacity: !root.aimed || box.isAimed ? 1 : 0.55
            scale: box.isAimed ? 1.05 : 1.0

            x: {
                if (box.modelData === "east")
                    return root.anchor.x + root.targetWidth / 2 + root.gap;
                if (box.modelData === "west")
                    return root.anchor.x - root.targetWidth / 2 - root.gap - box.width;
                return root.anchor.x - box.width / 2;
            }
            y: {
                if (box.modelData === "south")
                    return root.anchor.y + root.targetHeight / 2 + root.gap;
                if (box.modelData === "north")
                    return root.anchor.y - root.targetHeight / 2 - root.gap - box.height;
                return root.anchor.y - box.height / 2;
            }

            Behavior on tint {
                ColorAnimation {
                    duration: Theme.motion.micro
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }
            Behavior on opacity {
                NumberAnimation { duration: Theme.motion.micro }
            }
            Behavior on scale {
                NumberAnimation {
                    duration: Theme.motion.micro
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }

            Text {
                id: boxLabel
                anchors.centerIn: parent
                text: box.slot && box.slot.label ? box.slot.label : "—"
                color: box.isAimed ? Palette.onPrimaryContainer : Palette.onSurfaceVariant
                font.family: "monospace"
                font.pixelSize: root.boxFont
            }
        }
    }
}
