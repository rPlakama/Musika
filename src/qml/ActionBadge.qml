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
// along with this program; if not, see <https://www.gnu.org/licenses/>.

import QtQuick
import "Theme.js" as Theme

// Transient action feedback in the lower-right corner. The first action shows
// its name; repeating the same action during the hold window accumulates a
// numeric count (`Skip ×2`) and restarts the window.
Item {
    id: root

    property string label: ""
    property int count: 0
    property bool active: false
    property int holdMs: 1600

    width: badge.width
    height: badge.height
    opacity: 0
    visible: opacity > 0.01
    clip: true

    function labelFor(action) {
        switch (action) {
        case "next_track": return "Skip";
        case "prev_track": return "Previous";
        case "seek_back": return "Seek -5s";
        case "seek_forward": return "Seek +5s";
        case "volume_up": return "Volume +";
        case "volume_down": return "Volume -";
        case "play_pause": return "Play/Pause";
        case "repeat_cycle": return "Repeat";
        case "mute": return "Mute";
        case "refresh": return "Refresh";
        case "search":
        case "search_action": return "Search";
        case "enter": return "Open";
        case "back": return "Back";
        case "toggle_fullscreen": return "Fullscreen";
        case "move_up": return "Up";
        case "move_down": return "Down";
        case "move_left": return "Left";
        case "move_right": return "Right";
        case "top": return "Top";
        case "bottom": return "Bottom";
        case "page_up": return "Page Up";
        case "page_down": return "Page Down";
        case "visual": return "Select";
        case "visual_all": return "Select All";
        case "yank": return "Yank";
        case "paste": return "Paste";
        case "queue_toggle": return "Queue";
        case "scope_next": return "Next Scope";
        case "scope_prev": return "Previous Scope";
        case "order_next": return "Next Order";
        case "order_prev": return "Previous Order";
        case "layout_toggle": return "Layout";
        case "pane_cycle": return "Next Pane";
        case "pane_cycle_back": return "Previous Pane";
        default:
            return action.replace(/_/g, " ").replace(/\b\w/g,
                function (c) { return c.toUpperCase(); });
        }
    }

    function showAction(action, amount) {
        var next = labelFor(action);
        var value = Math.max(1, amount === undefined ? 1 : amount);
        if (active && label === next && dismiss.running)
            count += value;
        else {
            label = next;
            count = value;
        }
        active = true;
        opacity = 1;
        dismiss.restart();
    }

    Rectangle {
        id: badge
        height: Math.max(24, root.parent ? root.parent.height * 0.038 : 24)
        width: row.implicitWidth + height * 0.55
        radius: Theme.radius
        color: Qt.rgba(0, 0, 0, 0.55)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.12)

        Row {
            id: row
            anchors.centerIn: parent
            spacing: height * 0.12

            Text {
                text: root.label
                color: "white"
                font.family: Theme.fonts.body
                font.pixelSize: badge.height * 0.42
            }

            Text {
                visible: root.count > 1
                text: root.count > 1 ? "×" + root.count : ""
                color: "white"
                font.family: Theme.fonts.body
                font.bold: true
                font.pixelSize: badge.height * 0.42
            }
        }
    }

    Timer {
        id: dismiss
        interval: root.holdMs
        onTriggered: {
            root.active = false;
            root.opacity = 0;
        }
    }

    Behavior on opacity {
        NumberAnimation {
            duration: Theme.motion.quick
            easing.type: Easing.Bezier
            easing.bezierCurve: Theme.ease.standard
        }
    }
}
