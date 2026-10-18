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
import QtQuick.Particles
import "Theme.js" as Theme

// UI-60 "bright": the rating step's reward — a few stars shoot out of the
// favourite mark's centre in every direction, shrinking and fading as they
// fly. The system lives above the Alt overlay (so the burst outlives the
// tile's collapse) and is triggered per rating step.
Item {
    id: root

    /// Burst origin in this item's coordinates (the heart's centre).
    property real originX: width / 2
    property real originY: height / 2
    /// Stars per step ("few" — the rating key is tapped, not held).
    property int amount: 10
    property color starColor: Theme.favoriteStar
    /// The particle system only runs while the burst (plus its tail) is
    /// alive; an idle overlay costs no simulation.
    property bool _alive: false

    function burst() {
        root._alive = true;
        idle.stop();
        idle.restart();
        emitter.burst(root.amount)
    }

    Timer {
        id: idle
        interval: 1200
        repeat: false
        onTriggered: root._alive = false
    }

    function burstAt(x, y) {
        root.originX = x;
        root.originY = y;
        root.burst();
    }

    ParticleSystem {
        id: system
        anchors.fill: parent
        running: root._alive
    }

    ImageParticle {
        system: system
        source: "qrc:///particleresources/star.png"
        color: root.starColor
        colorVariation: 0.2
        alpha: 1.0
        entryEffect: ImageParticle.None
        autoRotation: true
        rotationVelocityVariation: 180
    }

    Emitter {
        id: emitter
        system: system
        x: root.originX
        y: root.originY
        width: 1
        height: 1
        enabled: false
        lifeSpan: 700
        lifeSpanVariation: 200
        size: 18
        endSize: 2
        sizeVariation: 6
        velocity: AngleDirection {
            angle: 0
            angleVariation: 360
            magnitude: 130
            magnitudeVariation: 60
        }
        acceleration: AngleDirection {
            angle: 0
            angleVariation: 360
            magnitude: 30
        }
    }
}
