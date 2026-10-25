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

// Debug-only frame-time overlay. `FrameAnimation` gives the interval between
// rendered frames; keep the last ~10 s and show the instantaneous, top,
// bottom, 1% low and average values.
Item {
    id: root

    readonly property int sampleWindow: 600
    property var frameTimes: []
    property real currentFps: 0
    property real topFps: 0
    property real minFps: 0
    property real lowOnePercentFps: 0
    property real averageFps: 0
    property real averageMs: 0
    property real delayMs: 0
    property int framesSinceUpdate: 0
    // When true, the debug overlay stops driving frames so the idle pacer
    // can settle the window at ~30 Hz.
    property bool idle: false

    width: 190
    height: column.implicitHeight + 12

    Rectangle {
        anchors.fill: parent
        radius: 4
        color: Qt.rgba(0, 0, 0, 0.34)
        border.width: 1
        border.color: Qt.rgba(1, 1, 1, 0.14)
    }

    Column {
        id: column
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.margins: 6
        spacing: 1

        Text {
            text: "Current: " + format(root.currentFps, 1) + " FPS"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "Top: " + format(root.topFps, 1) + " FPS"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "Min: " + format(root.minFps, 1) + " FPS"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "Low 1%: " + format(root.lowOnePercentFps, 1) + " FPS"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "Average: " + format(root.averageFps, 1) + " FPS"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "AverageMS: " + format(root.averageMs, 2) + " ms"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
        Text {
            text: "Delay: " + format(root.delayMs, 2) + " ms"
            color: "white"; font.pixelSize: 11; font.family: "monospace"
        }
    }

    FrameAnimation {
        running: root.visible && !root.idle
        onTriggered: {
            // FrameAnimation.frameTime is seconds, not milliseconds.
            var elapsed = frameTime * 1000;
            if (!(elapsed > 0) || elapsed > 1000)
                return;
            root.delayMs = elapsed;
            root.currentFps = 1000 / elapsed;
            root.frameTimes.push(elapsed);
            if (root.frameTimes.length > root.sampleWindow)
                root.frameTimes.shift();
            root.framesSinceUpdate++;
            if (root.framesSinceUpdate >= 15) {
                root.framesSinceUpdate = 0;
                root.updateStats();
            }
        }
    }

    // Trim trailing zeros: 60.0 FPS -> 60 FPS, 16.70 ms -> 16.7 ms.
    function format(value, digits) {
        return Number(value).toFixed(digits)
            .replace(/\.0+$/, "")
            .replace(/(\.\d*?)0+$/, "$1");
    }

    function updateStats() {
        var n = frameTimes.length;
        if (n === 0)
            return;
        var sum = 0;
        var fastest = Number.MAX_VALUE;
        var slowest = 0;
        for (var i = 0; i < n; i++) {
            var value = frameTimes[i];
            sum += value;
            fastest = Math.min(fastest, value);
            slowest = Math.max(slowest, value);
        }
        averageMs = sum / n;
        averageFps = 1000 / averageMs;
        topFps = 1000 / fastest;
        minFps = 1000 / slowest;

        // 1% low: mean FPS of the slowest 1% of frame intervals.
        var ordered = frameTimes.slice().sort(function (a, b) { return b - a; });
        var count = Math.max(1, Math.floor(n / 100));
        var lowSum = 0;
        for (var j = 0; j < count; j++)
            lowSum += ordered[j];
        lowOnePercentFps = 1000 / (lowSum / count);
    }
}
