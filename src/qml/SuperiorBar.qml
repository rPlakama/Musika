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
import QtQuick.Layouts
import QtQuick.VectorImage
import musika
import "Theme.js" as Theme

// Top status bar: identity, sync state/progress and the maintenance actions.
// Three panes share the width automatically — identity (left) and the sync
// report (right 1) split the leftover space, while the controls (right 2:
// progress track, sync, trashcan, action hint) size themselves.
Rectangle {
    radius: Theme.radius
    id: root

    // True while the status region owns the cursor (VI-17).
    property bool active: false
    /// Tab scope: which control Enter activates (0 = sync, 1 = trashcan).
    /// The ring marks it while the status region owns the cursor.
    property int scope: 0
    /// UI-57: the shared width ramp — identity and report type shrink with
    /// the window instead of only with its height.
    readonly property real widthScale: Theme.widthScale(root.width)
    /// The trashcan: the wipe menu's anchor (a click, or Alt while it is
    /// scoped, asks Home to arm it).
    readonly property Item wipeAnchor: wipeButton

    /// `sticky` = the menu stays armed after the trigger (mouse click path);
    /// the Alt-hold path passes false so release-over-neutral cancels.
    signal wipeRequested(var target, bool sticky)

    color: Palette.surfaceContainerLow

    // Human name for the ping envelope's `type` (`navidrome` → "Navidrome").
    function serverName() {
        if (Backend.serverType.length === 0)
            return "";
        return Backend.serverType.charAt(0).toUpperCase() + Backend.serverType.slice(1);
    }

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

    function handleAction(action) {
        if (action === "pane_cycle" || action === "pane_cycle_back") {
            // Two scopes: both directions land on the other one.
            root.scope = (root.scope + 1) % 2;
            return;
        }
        if (action !== "enter")
            return;
        if (root.scope === 0)
            Backend.syncLibrary();
        else
            root.wipeRequested(wipeButton, true);
    }

    /// ARC-UI-2: the search contract — the status region's interactable
    /// controls (the sync button and the trashcan). A jump moves the scope
    /// ring onto the control; Enter (the region's context-enter) then fires
    /// it exactly as Tab-scoping would.
    function searchRows() {
        return [["Sync"], ["Wipe"]];
    }

    function searchJump(row) {
        if (row < 0 || row > 1)
            return;
        root.scope = row;
    }

    /// Alt-menu availability: the reusable conditional hosts check before
    /// arming (`view.altActionsAvailable !== false`). The status bar only
    /// offers the wipe quadrants while the trashcan is scoped — the sync
    /// button has no Alt action.
    readonly property bool altActionsAvailable: root.scope === 1

    /// Alt hold: the same menu a trashcan click opens, when available.
    function armQuickActions() {
        if (!root.altActionsAvailable)
            return;
        root.wipeRequested(wipeButton, false);
    }

    // UI-49 bounce-back: on unhover without a tap, the same shake as the
    // denied-input cue in search, without the red glyph flash.
    ShakeFx {
        id: syncFx
        target: syncIcon
    }

    // Bottom divider.
    Rectangle {
        radius: Theme.radius
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Theme.hairline(root.height)
        color: Palette.outlineVariant
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: root.width * 0.012
        anchors.rightMargin: root.width * 0.012
        spacing: root.width * 0.015

        // -------------------------------------------------------------------
        // Left pane: identity. Takes half the leftover width.
        // -------------------------------------------------------------------
        Rectangle {
            radius: Theme.radius
            id: identityPane
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: "transparent"

            FadeText {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                horizontalAlignment: Text.AlignLeft
                // CACHE-9: offline mode greets as "Offline, <user>".
                text: Backend.username.length > 0
                    ? (Backend.offline ? "Offline, " : "Welcome, ") + Backend.username
                    : (Backend.offline ? "Offline" : "")
                color: Backend.offline ? Palette.tertiary : Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("title", root.height) * 1.5 * root.widthScale
                elide: Text.ElideRight
                animated: false
            }
        }

        // -------------------------------------------------------------------
        // Right 1: the sync report. Takes the other half, right-aligned so it
        // sits beside the controls.
        // -------------------------------------------------------------------
        Rectangle {
            radius: Theme.radius
            id: reportPane
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: "transparent"

            FadeText {
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                horizontalAlignment: Text.AlignRight
                text: {
                    var status = Backend.librarySyncing ? "Syncing…" : root.lastSyncText();
                    var provider = root.serverName();
                    return "Last Sync: " + status + (provider.length > 0 ? ", " + provider : "");
                }
                color: Backend.librarySyncing ? Palette.tertiary : Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("title", root.height) * 1.35 * root.widthScale
                elide: Text.ElideRight
            }
        }

        // -------------------------------------------------------------------
        // Right 1.5: the focus hint, in its own box (VI-17). It swaps width
        // on Tab (sync vs wipe), so it must not live inside the controls: as
        // a sibling, its width spends the report's slack and the buttons —
        // with the scope ring riding them — never move. A safe margin keeps
        // it clear of the buttons.
        // -------------------------------------------------------------------
        Rectangle {
            radius: Theme.radius
            id: hintPane
            visible: root.active
            Layout.preferredWidth: root.active ? hintText.implicitWidth : 0
            Layout.rightMargin: root.width * 0.012
            Layout.fillHeight: true
            color: "transparent"

            FadeText {
                id: hintText
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                text: (root.scope === 0
                    ? "Enter: sync"
                    : "Enter: wipe · Alt: menu")
                    + " · Tab: cycle"
                color: Palette.onSurfaceVariant
                font.pixelSize: Theme.fontSize("label", root.height)
            }
        }

        // -------------------------------------------------------------------
        // Right 2: the controls, sized by their own contents.
        // -------------------------------------------------------------------
        Rectangle {
            radius: Theme.radius
            id: controlPane
            Layout.preferredWidth: controls.implicitWidth
            Layout.fillHeight: true
            color: "transparent"

            RowLayout {
                id: controls
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                spacing: root.width * 0.015

                // Determinate progress across the last reported stage.
                // Always collapsed to a 2px tick; the sync glyph carries
                // the running state.
                Rectangle {
                    radius: Theme.radius
                    id: track
                    Layout.preferredWidth: 2
                    Layout.preferredHeight: Math.max(2, root.height * 0.16)
                    color: Palette.outlineVariant
                    clip: true

                    Rectangle {
                        radius: Theme.radius
                        width: track.width * Math.max(0, Math.min(1, Backend.syncProgress))
                        height: parent.height
                        color: Backend.librarySyncing ? Palette.tertiary : Palette.primary

                        Behavior on width {
                            NumberAnimation {
                                id: progressAnim
                                duration: Theme.motion.quick
                                easing.type: Easing.Linear
                                // Spin follows the bar: when the fill settles
                                // after the last progress step, the icon winds
                                // down with it.
                                onRunningChanged: syncButton.updateSpin()
                            }
                        }
                    }
                }

                // Sync button (roadmap D-6): the frame is invisible but keeps
                // the hit area/proportions and the static scope ring; only the
                // glyph spins.
                Rectangle {
                    id: syncButton
                    Layout.preferredWidth: root.height * 0.6
                    Layout.preferredHeight: root.height * 0.6
                    radius: Theme.radius
                    color: "transparent"

                    // Set on tap so an unhover after asking for a sync never
                    // bounces back over the sync spin.
                    property bool tapped: false

                    // Spin while the progress bar is moving (a sync runs / the
                    // fill is still settling) and on hover (slow). The glyph
                    // and the bar are coupled: when the bar stops, the spin
                    // stops too; restarting resumes from the current angle;
                    // `settle()` puts it back when a hover ends untapped.
                    function spinning() {
                        return Backend.librarySyncing || progressAnim.running || syncHover.hovered;
                    }

                    function updateSpin() {
                        if (spinning()) {
                            spin.stop();
                            settleSpin.stop();
                            spin.from = syncIcon.rotation;
                            spin.to = spin.from + 360;
                            spin.duration = Backend.librarySyncing ? Theme.motion.spin : Theme.motion.spinSlow;
                            spin.start();
                        } else {
                            spin.stop();
                            // A multiple of 360 is the same angle: bound the
                            // value without moving the glyph.
                            syncIcon.rotation = syncIcon.rotation % 360;
                        }
                    }

                    // Hover left without a tap: shake and bounce the glyph
                    // back to its original angle instead of parking it
                    // mid-spin.
                    function settle() {
                        settleSpin.stop();
                        settleSpin.from = syncIcon.rotation;
                        settleSpin.to = Math.round(syncIcon.rotation / 360) * 360;
                        settleSpin.start();
                        syncFx.shake();
                    }

                    NumberAnimation {
                        id: spin
                        target: syncIcon
                        property: "rotation"
                        loops: Animation.Infinite
                        easing.type: Easing.Linear
                    }

                    Anim {
                        id: settleSpin
                        target: syncIcon
                        property: "rotation"
                        duration: Theme.motion.standard
                        easing.bezierCurve: Theme.ease.bounce
                    }

                    Connections {
                        target: Backend
                        function onLibrary_syncing_changed() {
                            syncButton.updateSpin();
                        }
                    }

                    SymbolIcon {
                        id: syncIcon
                        anchors.centerIn: parent
                        source: "qrc:/app/icons/reload-ui-2-svgrepo-com.svg"
                        symbolColor: Palette.onSurfaceVariant
                        width: root.height * 0.26
                        height: width
                        opacity: Backend.librarySyncing ? 0.38 : (syncHover.hovered ? 1.0 : 0.85)
                        // UI-49: untapped unhover bounce, a transform so
                        // nothing shifts layout.
                        transform: Translate { x: syncFx.offset }

                        Behavior on opacity { Anim { duration: Theme.motion.micro } }
                    }

                    ScopeRing {
                        shown: root.active && root.scope === 0
                    }

                    // Clickable affordance (reuse for every clickable): the
                    // hover handler carries the pointing-hand cursor, the tap
                    // handler the action.
                    HoverHandler {
                        id: syncHover
                        cursorShape: Qt.PointingHandCursor
                        onHoveredChanged: {
                            if (hovered) {
                                syncButton.tapped = false;
                                syncButton.updateSpin();
                            } else {
                                syncButton.updateSpin();
                                if (!syncButton.tapped && !syncButton.spinning() && !Theme.reduceMotion)
                                    syncButton.settle();
                            }
                        }
                    }
                    TapHandler {
                        enabled: !Backend.librarySyncing
                        onTapped: {
                            root.scope = 0;
                            syncButton.tapped = true;
                            Backend.syncLibrary();
                        }
                    }
                }

                // Trashcan: opens the wipe quadrant menu (Home arms the shared
                // Alt overlay in sticky mode; h/j/k/l or arrows aim, Enter
                // fires, Esc cancels). Disabled while a sync writes the
                // databases.
                Rectangle {
                    id: wipeButton
                    Layout.preferredWidth: root.height * 0.6
                    Layout.preferredHeight: root.height * 0.6
                    radius: Theme.radius
                    color: "transparent"

                    SymbolIcon {
                        anchors.centerIn: parent
                        source: "qrc:/app/icons/delete-outline.svg"
                        symbolColor: wipeHover.hovered ? Palette.error : Palette.onSurfaceVariant
                        width: root.height * 0.26
                        height: width
                        opacity: Backend.librarySyncing ? 0.38 : (wipeHover.hovered ? 1.0 : 0.85)

                        Behavior on opacity { Anim { duration: Theme.motion.micro } }
                    }

                    ScopeRing {
                        shown: root.active && root.scope === 1
                    }

                    HoverHandler {
                        id: wipeHover
                        cursorShape: Qt.PointingHandCursor
                    }
                    TapHandler {
                        enabled: !Backend.librarySyncing
                        onTapped: {
                            root.scope = 1;
                            root.wipeRequested(wipeButton, true);
                        }
                    }
                }

            }
        }
    }
}
