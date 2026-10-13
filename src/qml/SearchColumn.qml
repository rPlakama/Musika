import QtQuick
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
                color: root.active ? Theme.primary : Theme.onSurfaceVariant
                font.pixelSize: root.headerH * 0.42
                font.bold: root.active
            }

            FadeText {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: root.model ? root.model.count : 0
                color: root.active ? Theme.primary : Theme.onSurfaceVariant
                font.pixelSize: root.headerH * 0.36
            }

            Rectangle {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                height: Math.max(1, parent.height * Theme.hairlineRatio)
                color: root.active ? Theme.primary : Theme.outlineVariant
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
                    height: root.height * 0.115
                    color: index === root.cursor ? Theme.secondaryContainer : "transparent"

                    Rectangle {
                        id: thumb
                        anchors.left: parent.left
                        anchors.verticalCenter: parent.verticalCenter
                        width: row.height * 0.72
                        height: row.height * 0.72
                        radius: root.roundedArt ? width * 0.5 : Theme.radius
                        color: Theme.surfaceContainerHighest
                        clip: true

                        Image {
                            anchors.fill: parent
                            source: row.cover
                            fillMode: Image.PreserveAspectCrop
                            asynchronous: true
                            visible: status === Image.Ready
                        }
                    }

                    FadeText {
                        anchors.left: thumb.right
                        anchors.leftMargin: row.height * 0.18
                        anchors.right: metaLabel.left
                        anchors.rightMargin: row.height * 0.18
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.subtitle.length > 0 ? row.title + "   " + row.subtitle : row.title
                        color: index === root.cursor ? Theme.onSecondaryContainer : Theme.onSurface
                        font.pixelSize: row.height * 0.34
                        elide: Text.ElideRight
                    }

                    FadeText {
                        id: metaLabel
                        anchors.right: parent.right
                        anchors.rightMargin: row.height * 0.18
                        anchors.verticalCenter: parent.verticalCenter
                        text: row.meta
                        color: Theme.onSurfaceVariant
                        font.pixelSize: row.height * 0.28
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
                color: Theme.onSurfaceVariant
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
                        color: Theme.surfaceContainer
                        opacity: 0.6
                    }
                }
            }
        }
    }
}
