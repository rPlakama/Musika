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
import musika
import "Theme.js" as Theme

// One column of the floating search: a header (title + count) over the result
// list, with "no matches" and a loading skeleton underneath.
Item {
    id: root

    property string title: ""
    property bool roundedArt: false
    property var model: null
    property int cursor: -1
    property bool active: false
    property bool loading: false
    property bool hasQuery: false

    signal hovered(int index)
    signal picked(int index)

    readonly property real headerH: height * 0.10

    Column {
        anchors.fill: parent
        spacing: 0

        Item {
            id: header
            width: parent.width
            height: root.headerH

            FadeText {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                text: root.title
                color: root.active ? Palette.primary : Palette.onSurfaceVariant
                font.pixelSize: root.headerH * 0.42
                font.bold: root.active
            }

            FadeText {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: root.model ? root.model.count : 0
                color: root.active ? Palette.primary : Palette.onSurfaceVariant
                font.pixelSize: root.headerH * 0.36
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Math.max(1, parent.height * Theme.hairlineRatio)
                color: root.active ? Palette.primary : Palette.outlineVariant
            }
        }

        Item {
            width: parent.width
            height: parent.height - root.headerH

            ListView {
                id: list
                anchors.fill: parent
                clip: true
                model: root.model
                currentIndex: root.cursor
                boundsBehavior: Flickable.StopAtBounds

                onCurrentIndexChanged: {
                    if (currentIndex >= 0)
                        positionViewAtIndex(currentIndex, ListView.Contain);
                }

                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string title
                    required property string subtitle
                    required property string meta
                    required property string cover

                    width: list.width
                    height: root.height * 0.12
                    color: index === root.cursor ? Palette.secondaryContainer : "transparent"

                    Behavior on color {
                        ColorAnimation { duration: Theme.motion.quick; easing.type: Easing.Bezier; easing.bezierCurve: Theme.ease.standard }
                    }

                    Rectangle {
                        id: thumb
                        anchors.left: parent.left
                        anchors.verticalCenter: parent.verticalCenter
                        width: row.height * 0.72
                        height: row.height * 0.72
                        radius: root.roundedArt ? width * 0.5 : Theme.radius
                        color: Palette.surfaceContainerHighest
                        clip: true

                        Image {
                            anchors.fill: parent
                            source: row.cover
                            fillMode: Image.PreserveAspectCrop
                            asynchronous: true
                            visible: status === Image.Ready
                        }
                    }

                    // Two-line cell: title over a muted subtitle, never one
                    // string in one colour.
                    Column {
                        anchors.left: thumb.right
                        anchors.leftMargin: row.height * 0.18
                        anchors.right: metaLabel.left
                        anchors.rightMargin: row.height * 0.18
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: row.height * 0.04

                        FadeText {
                            width: parent.width
                            text: row.title
                            color: index === root.cursor ? Palette.onSecondaryContainer : Palette.onSurface
                            font.pixelSize: Theme.clamp(row.height * 0.3, 12, 18)
                            elide: Text.ElideRight
                            animated: false
                        }

                        FadeText {
                            width: parent.width
                            visible: row.subtitle.length > 0
                            text: row.subtitle
                            color: index === root.cursor ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                            font.pixelSize: Theme.clamp(row.height * 0.24, 10, 14)
                            elide: Text.ElideRight
                            animated: false
                        }
                    }

                    FadeText {
                        id: metaLabel
                        anchors.right: parent.right
                        anchors.rightMargin: row.height * 0.18
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.meta
                        color: index === root.cursor ? Palette.onSecondaryContainer : Palette.onSurfaceVariant
                        font.pixelSize: Theme.clamp(row.height * 0.26, 10, 14)
                        animated: false
                    }

                    HoverHandler {
                        onHoveredChanged: {
                            if (hovered)
                                root.hovered(row.index);
                        }
                    }

                    TapHandler {
                        onTapped: root.picked(row.index)
                    }
                }
            }

            FadeText {
                anchors.centerIn: parent
                visible: root.hasQuery && !root.loading && root.model && root.model.count === 0
                text: "no matches"
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.055
            }

            // Skeleton while the request is in flight.
            Column {
                anchors.fill: parent
                anchors.margins: root.height * 0.03
                spacing: root.height * 0.025
                visible: root.loading && root.model && root.model.count === 0

                Repeater {
                    model: 4

                    delegate: Rectangle {
                        required property int index

                        width: parent.width * (1 - index * 0.12)
                        height: root.height * 0.075
                        radius: Theme.radius
                        color: Palette.surfaceContainer
                        opacity: 0.6
                    }
                }
            }
        }
    }
}
