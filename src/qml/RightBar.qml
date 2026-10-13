import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Top status bar: indexed counts, sync state/progress and a sync action.
Rectangle {
    id: root

    color: Theme.surfaceContainerLow

    // Relative "last sync" label: Now / HH:mm / dd / Nw / Nmo / Ny.
    function lastSyncText() {
        var seconds = Number(Backend.lastSync);
        if (!Backend.lastSync || !seconds)
            return "—";

        var then = new Date(seconds * 1000);
        var diff = (Date.now() - then.getTime()) / 1000; // seconds ago
        if (diff < 60)
            return "Now";

        var hours = diff / 3600;
        if (hours < 24)
            return Qt.formatDateTime(then, "HH:mm");

        var days = hours / 24;
        if (days < 7)
            return Qt.formatDateTime(then, "dd");

        var weeks = days / 7;
        if (weeks < 4)
            return Math.floor(weeks) + "w";

        var months = days / 30;
        if (months < 12)
            return Math.floor(months) + "mo";

        return Math.floor(days / 365) + "y";
    }

    // The status panel has no cursor of its own; Enter triggers a sync.
    function handleAction(action) {
        if (action === "enter")
            Backend.syncLibrary();
    }

    // Bottom divider.
    Rectangle {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Math.max(1, root.height * Theme.hairlineRatio)
        color: Theme.outlineVariant
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.012
        anchors.rightMargin: root.width * 0.012
        spacing: root.width * 0.015

        // Mode chip (N4 placeholder): always shows the mode; NORMAL until a
        // global mode machine exists.
        Rectangle {
            implicitWidth: modeLabel.implicitWidth + root.height * 0.36
            implicitHeight: root.height * 0.5
            radius: Theme.radius
            color: Theme.surfaceContainerHighest
            border.color: Theme.primary
            border.width: Math.max(1, root.height * Theme.hairlineRatio)

            FadeText {
                id: modeLabel
                anchors.centerIn: parent
                text: "NORMAL"
                color: Theme.primary
                font.pixelSize: Theme.fontSize("label", root.height)
                font.bold: true
                font.letterSpacing: root.height * 0.025
                animated: false
            }
        }

        Item {
            Layout.fillWidth: true
        }

        FadeText {
            text: Backend.librarySyncing ? "Syncing…" : "Up to date"
            color: Backend.librarySyncing ? Theme.tertiary : Theme.onSurfaceVariant
            font.pixelSize: Theme.fontSize("label", root.height)
        }

        // Determinate progress across the last reported stage.
        Rectangle {
            id: track
            Layout.preferredWidth: root.width * 0.12
            Layout.preferredHeight: Math.max(2, root.height * 0.16)
            color: Theme.outlineVariant

            Rectangle {
                width: track.width * Math.max(0, Math.min(1, Backend.syncProgress))
                height: parent.height
                color: Backend.librarySyncing ? Theme.tertiary : Theme.primary

                Behavior on width {
                    NumberAnimation {
                        duration: Theme.motion.quick
                        easing.type: Easing.Linear
                    }
                }
            }
        }

        Rectangle {
            id: syncButton
            Layout.preferredWidth: syncLabel.implicitWidth + root.height * 0.4
            Layout.preferredHeight: root.height * 0.6
            radius: Theme.radius
            color: !Backend.librarySyncing && syncHover.hovered
                ? Theme.primary
                : Theme.primaryContainer

            FadeText {
                id: syncLabel
                anchors.centerIn: parent
                text: "Sync"
                color: Backend.librarySyncing ? Theme.stateDisabledContent : Theme.onPrimaryContainer
                font.pixelSize: Theme.fontSize("label", root.height)
            }

            HoverHandler {
                id: syncHover
                enabled: !Backend.librarySyncing
            }
            TapHandler {
                enabled: !Backend.librarySyncing
                onTapped: Backend.syncLibrary()
            }
        }

        FadeText {
            text: "Last: " + root.lastSyncText()
            color: Theme.onSurfaceVariant
            font.pixelSize: Theme.fontSize("label", root.height)
        }
    }
}
