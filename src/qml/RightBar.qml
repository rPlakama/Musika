import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Right bar: a read-only status readout for the library.
//
// Shows the indexed counts, whether a sync is running, how far along it is and
// when it last ran. Clicking anywhere on the panel starts a sync (per design).
Rectangle {
    id: root

    color: Theme.surfaceContainerLow

    function lastSyncText() {
        var seconds = Number(Backend.lastSync);
        if (!Backend.lastSync || !seconds)
            return "—";
        return new Date(seconds * 1000).toLocaleString();
    }

    // Left divider.
    Rectangle {
        anchors.left: parent.left
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        width: Math.max(1, root.width * Theme.hairlineRatio)
        color: Theme.outlineVariant
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: root.width * 0.08
        spacing: root.width * 0.06

        Text {
            text: "LIBRARY"
            color: Theme.primary
            font.pixelSize: root.width * 0.06
            font.bold: true
            font.letterSpacing: root.width * 0.01
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: root.width * 0.03

            Repeater {
                model: [
                    { label: "Artists", value: Backend.artistCount },
                    { label: "Albums", value: Backend.albumCount },
                    { label: "Songs", value: Backend.songCount }
                ]

                delegate: RowLayout {
                    id: stat
                    required property var modelData
                    readonly property real fs: width * 0.074
                    Layout.fillWidth: true

                    Text {
                        Layout.fillWidth: true
                        text: stat.modelData.label
                        color: Theme.onSurfaceVariant
                        font.pixelSize: stat.fs
                    }
                    Text {
                        text: stat.modelData.value
                        color: Theme.onSurface
                        font.pixelSize: stat.fs
                        font.bold: true
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(1, root.width * Theme.hairlineRatio)
            color: Theme.outlineVariant
        }

        Text {
            text: "SYNC"
            color: Theme.primary
            font.pixelSize: root.width * 0.06
            font.bold: true
            font.letterSpacing: root.width * 0.01
        }

        Text {
            Layout.fillWidth: true
            text: Backend.librarySyncing ? "Syncing…" : "Up to date"
            color: Backend.librarySyncing ? Theme.tertiary : Theme.onSurfaceVariant
            font.pixelSize: root.width * 0.062
        }

        Text {
            Layout.fillWidth: true
            text: Backend.syncMessage
            color: Theme.onSurfaceVariant
            font.pixelSize: root.width * 0.05
            elide: Text.ElideRight
        }

        // Determinate progress across the last reported stage.
        Rectangle {
            id: track
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(2, root.height * 0.007)
            color: Theme.outlineVariant

            Rectangle {
                width: track.width * Math.max(0, Math.min(1, Backend.syncProgress))
                height: parent.height
                color: Backend.librarySyncing ? Theme.tertiary : Theme.primary
                Behavior on width {
                    NumberAnimation { duration: 150 }
                }
            }
        }

        Text {
            Layout.fillWidth: true
            text: "Last sync: " + root.lastSyncText()
            color: Theme.onSurfaceVariant
            font.pixelSize: root.width * 0.05
        }

        Item {
            Layout.fillHeight: true
        }

        Text {
            Layout.fillWidth: true
            text: Backend.librarySyncing ? "Syncing…" : "Click to sync"
            color: Theme.onSurfaceVariant
            font.pixelSize: root.width * 0.052
            horizontalAlignment: Text.AlignHCenter
        }
    }

    // The whole status panel is the sync affordance.
    TapHandler {
        enabled: !Backend.librarySyncing
        onTapped: Backend.syncLibrary()
    }
}
