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

// Elegant shutdown: fade the scrim in, spawn the white "Bye!" letters with a
// small bounce, then quit the Qt event loop. `qq` in Home calls `play()`.
Item {
    id: root

    property bool running: false
    visible: false
    opacity: 0
    clip: true

    function play() {
        if (running)
            return;
        running = true;
        visible = true;
        opacity = 0;
        fadeIn.restart();
        for (var i = 0; i < lettersRepeater.count; ++i) {
            var letter = lettersRepeater.itemAt(i);
            if (letter)
                letter.restart();
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Qt.rgba(0, 0, 0, 0.62)
    }

    Row {
        id: letters
        anchors.centerIn: parent
        spacing: Math.max(2, root.width * 0.008)

        Repeater {
            id: lettersRepeater
            model: ["B", "y", "e", "!"]

            delegate: Text {
                id: letter
                required property string modelData
                required property int index

                text: modelData
                color: "white"
                font.family: Theme.fonts.wordmark
                font.bold: true
                font.pixelSize: Math.max(42, Math.min(root.height * 0.14, 96))
                opacity: 0
                scale: 0.6

                function restart() {
                    opacity = 0;
                    scale = 0.6;
                    spawn.restart();
                }

                ParallelAnimation {
                    id: spawn
                    NumberAnimation {
                        target: letter
                        property: "opacity"
                        from: 0
                        to: 1
                        duration: 200
                        easing.type: Easing.Bezier
                        easing.bezierCurve: Theme.ease.standard
                    }
                    NumberAnimation {
                        target: letter
                        property: "scale"
                        from: 0.6
                        to: 1
                        duration: 200
                        easing.type: Easing.OutBack
                    }
                }
            }
        }
    }

    NumberAnimation {
        id: fadeIn
        target: root
        property: "opacity"
        from: 0
        to: 1
        duration: 200
        easing.type: Easing.Bezier
        easing.bezierCurve: Theme.ease.standard
        onFinished: {
            Backend.flushStatus();
            Qt.quit();
        }
    }
}
