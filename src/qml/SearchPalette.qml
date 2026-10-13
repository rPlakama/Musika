import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Floating search palette (Shift+Space+F). Results come from the FTS5 index
// (`search.db`); Enter plays the highlighted song, Esc closes.
Item {
    id: root

    property bool opened: false

    visible: opened
    z: 30

    function open() {
        opened = true;
        field.text = "";
        results.clear();
        list.currentIndex = -1;
        field.forceActiveFocus();
    }

    function close() {
        opened = false;
        Backend.releaseFocus();
    }

    function playCurrent() {
        var hit = results.get(list.currentIndex);
        if (hit) {
            Backend.playSong(hit.songId);
            root.close();
        }
    }

    function apply(payload) {
        results.clear();
        var hits = JSON.parse(payload);
        for (var i = 0; i < hits.length; i++)
            results.append({ songId: hits[i].id, label: hits[i].label });
        list.currentIndex = hits.length > 0 ? 0 : -1;
    }

    ListModel {
        id: results
    }

    Timer {
        id: debounce
        interval: 120
        onTriggered: {
            if (field.text.length > 0)
                Backend.requestPalette(field.text);
            else
                results.clear();
        }
    }

    Connections {
        target: Backend
        function onDataReady(channel, payload) {
            if (channel === "palette" && root.opened)
                root.apply(payload);
        }
    }

    // Dim backdrop; click outside to dismiss.
    Rectangle {
        anchors.fill: parent
        color: "#000000"
        opacity: 0.5

        TapHandler {
            onTapped: root.close()
        }
    }

    Rectangle {
        id: panel
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: root.height * 0.1
        width: root.width * 0.5
        height: root.height * 0.6
        color: Theme.surfaceContainerHigh
        radius: Theme.radius
        border.color: Theme.primary
        border.width: Math.max(1, root.height * Theme.hairlineRatio)

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: root.width * 0.012
            spacing: root.height * 0.012

            TextField {
                id: field
                Layout.fillWidth: true
                placeholderText: "Search songs…"
                placeholderTextColor: Theme.onSurfaceVariant
                color: Theme.onSurface
                font.pixelSize: root.height * 0.028
                background: Rectangle {
                    color: Theme.surfaceContainerHighest
                    radius: Theme.radius
                    border.color: field.activeFocus ? Theme.primary : Theme.outlineVariant
                    border.width: Math.max(1, root.height * Theme.hairlineRatio)
                }

                onTextChanged: debounce.restart()

                Keys.onDownPressed: list.currentIndex = Math.min(results.count - 1, list.currentIndex + 1)
                Keys.onUpPressed: list.currentIndex = Math.max(0, list.currentIndex - 1)
                Keys.onReturnPressed: root.playCurrent()
                Keys.onEnterPressed: root.playCurrent()
                Keys.onEscapePressed: root.close()
            }

            ListView {
                id: list
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: results
                boundsBehavior: Flickable.StopAtBounds
                currentIndex: -1

                delegate: Rectangle {
                    id: row
                    required property int index
                    required property string songId
                    required property string label

                    width: list.width
                    height: root.height * 0.045
                    color: index === list.currentIndex
                        ? Theme.secondaryContainer
                        : (index % 2 === 0 ? Theme.surfaceContainerHigh : Theme.surfaceContainer)

                    FadeText {
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.left: parent.left
                        anchors.leftMargin: root.width * 0.01
                        anchors.right: parent.right
                        anchors.rightMargin: root.width * 0.01
                        text: row.label
                        color: index === list.currentIndex ? Theme.onSecondaryContainer : Theme.onSurface
                        font.pixelSize: root.height * 0.024
                        elide: Text.ElideRight
                    }

                    TapHandler {
                        onTapped: {
                            list.currentIndex = row.index;
                            root.playCurrent();
                        }
                    }
                }
            }
        }
    }
}
