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
import QtQuick.Shapes
import musika

// The Navidrome mark, redrawn with QtQuick.Shapes so its negative-space
// segments (outer rim, label, grooves, centre pin) follow
// `Palette.background` instead of a hardcoded white — a VectorImage cannot be
// tinted. Geometry is the original 256x256 logo, scaled to the item; the
// blue is the brand colour and stays fixed on purpose (D-6 identity logo).
Shape {
    id: root

    antialiasing: true
    preferredRendererType: Shape.CurveRenderer

    transform: Scale {
        origin.x: 0
        origin.y: 0
        xScale: root.width / 256
        yScale: root.height / 256
    }

    // Blue disc and its background-coloured rim.
    ShapePath {
        fillColor: "#0084ff"
        strokeColor: Palette.background
        strokeWidth: 10.36
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 122.8; radiusY: 122.8
            startAngle: 0; sweepAngle: 360
        }
    }

    // Label.
    ShapePath {
        fillColor: Palette.background
        strokeColor: "transparent"
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 43.7; radiusY: 43.7
            startAngle: 0; sweepAngle: 360
        }
    }

    // Grooves: two opposite quarter arcs per radius (separate paths so no
    // chord is drawn between them).
    ShapePath {
        fillColor: "transparent"
        strokeColor: Palette.background
        strokeWidth: 10
        capStyle: ShapePath.RoundCap
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 73.8; radiusY: 73.8
            startAngle: 0; sweepAngle: 90
        }
    }
    ShapePath {
        fillColor: "transparent"
        strokeColor: Palette.background
        strokeWidth: 10
        capStyle: ShapePath.RoundCap
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 73.8; radiusY: 73.8
            startAngle: 180; sweepAngle: 90
        }
    }
    ShapePath {
        fillColor: "transparent"
        strokeColor: Palette.background
        strokeWidth: 10
        capStyle: ShapePath.RoundCap
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 96.5; radiusY: 96.5
            startAngle: 0; sweepAngle: 90
        }
    }
    ShapePath {
        fillColor: "transparent"
        strokeColor: Palette.background
        strokeWidth: 10
        capStyle: ShapePath.RoundCap
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 96.5; radiusY: 96.5
            startAngle: 180; sweepAngle: 90
        }
    }

    // Centre pin.
    ShapePath {
        fillColor: Palette.background
        strokeColor: "transparent"
        PathAngleArc {
            centerX: 128; centerY: 128
            radiusX: 9; radiusY: 9
            startAngle: 0; sweepAngle: 360
        }
    }
}
