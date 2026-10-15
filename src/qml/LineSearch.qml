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
import QtQuick.Controls
import QtQuick.Layouts
import musika
import "Theme.js" as Theme
import "SearchMatch.js" as Match

// ARC-UI-2 M1: the `/` in-surface search command line — a flat, full-width
// bar anchored to the bottom edge over a 50% black scrim (no blur). It
// matches the top surface's rendered row text (literal, smartcase), Tab /
// Shift-Tab cycle the matching rows (the cursor previews each in place) and
// Enter commits: the prompt closes and the surface's own context-Enter runs
// on the match — open, play, jump, whatever the row's enter action is. Esc
// abandons; a non-empty query with no matches flashes the denied cue and
// never moves the cursor (M2 adds the in-row block highlights).
Item {
    id: root

    property bool opened: false
    /// The surface being searched (`searchRows()`/`searchJump(row)`).
    property var target: null
    /// Every row's rendered cells, captured when the prompt opens.
    property var _rows: []
    /// Row indices whose cells contain the query, in surface order.
    property var _candidates: []
    property int _index: -1
    /// UI-49: the last zero query that already got the denied cue, so one
    /// response lights it once and holding a key never strobes.
    property string _zeroQuery: ""

    readonly property string counterText: (_candidates.length > 0
        && field.text.length > 0)
        ? "[" + (_index + 1) + "/" + _candidates.length + "]"
        : ""

    signal entered()

    function open(t) {
        opened = true;
        target = t;
        pop.play();
        // The global keymap dies while the command line types (see
        // `Backend::key_event`).
        Backend.setLineSearchOpen(true);
        _rows = (t && t.searchRows) ? t.searchRows() : [];
        _candidates = [];
        _index = -1;
        _zeroQuery = "";
        field.text = "";
        field.forceActiveFocus();
    }

    function close() {
        opened = false;
        pop.collapse();
        Backend.setLineSearchOpen(false);
        Backend.releaseFocus();
    }

    /// Esc while typing abandons the search (the preview cursor stays where
    /// the last match put it — Neovim restores, but the surface memory can
    /// do that later; M4).
    function abandon() {
        close();
    }

    /// Recompute the candidate ring for the typed query (the matcher lives in
    /// `SearchMatch.js`, unit-tested).
    function refresh() {
        var q = field.text;
        _candidates = Match.candidates(_rows, q);
        _index = -1;
        if (_candidates.length > 0) {
            // Live preview: the first match takes the cursor, Tab cycles.
            _index = 0;
            jump();
        } else if (q.length > 0 && q !== _zeroQuery) {
            _zeroQuery = q;
            denied();
        }
    }

    function jump() {
        if (_index < 0 || !target || !target.searchJump)
            return;
        target.searchJump(_candidates[_index]);
    }

    /// Cycle the candidate ring; Tab forward, Shift-Tab back, wrapping.
    function step(delta) {
        if (_candidates.length === 0)
            return;
        _index = (_index + delta + _candidates.length) % _candidates.length;
        jump();
    }

    /// Enter: commit on the current match — close, then let Home dispatch the
    /// surface's own context-Enter (VI-16) on the row the cursor now sits on.
    function commit() {
        if (_candidates.length > 0) {
            close();
            entered();
            return;
        }
        if (field.text.length > 0)
            denied();
        else
            close();
    }

    // Keymap actions that still reach Home while the prompt is up (IPC, or a
    // window key the field did not consume).
    function handleAction(action) {
        if (action === "enter")
            commit();
        else if (action === "back")
            abandon();
    }

    function denied() {
        if (!Theme.reduceMotion)
            deniedFlash.restart();
    }

    visible: opened || bar.opacity > 0
    z: Theme.zCommand

    // ARC-UI-2: the shared spawn FX (see `PopFx`).
    PopFx {
        id: pop
    }

    SequentialAnimation {
        id: deniedFlash
        NumberAnimation {
            target: deniedLine
            property: "opacity"
            to: 1
            duration: Theme.motion.flash
        }
        NumberAnimation {
            target: deniedLine
            property: "opacity"
            to: 0
            duration: Theme.motion.quick
        }
    }

    // The dim, not a blur: everything under the prompt drops to half light.
    Rectangle {
        id: scrim
        anchors.fill: parent
        color: "black"
        opacity: root.opened ? 0.5 : 0

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }
    }

    // The command line itself: full width, no radius, no border highlight —
    // one vertically centred line of prompt (Neovim's last row, reskinned).
    Rectangle {
        id: bar
        width: root.width
        height: root.height * 0.07
        y: root.height - height
        color: Palette.surface
        opacity: root.opened ? 1 : 0
        clip: true

        // ARC-UI-2: the shared spawn FX (the Alt overlay's spring) instead
        // of the old rise: the command line pops out of its bottom edge.
        transform: Scale {
            origin.x: bar.width / 2
            origin.y: bar.height
            xScale: pop.value
            yScale: pop.value
        }

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.motion.standard
                easing.type: Easing.Bezier
                easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
            }
        }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: root.width * 0.04
            anchors.rightMargin: root.width * 0.04
            spacing: root.height * 0.012

            // The Neovim command-line prefix.
            Text {
                text: "/"
                color: Palette.primary
                font.family: Theme.fonts.body
                font.pixelSize: bar.height * 0.42
                font.bold: true
                Layout.alignment: Qt.AlignVCenter
            }

            Item {
                Layout.fillWidth: true
                Layout.fillHeight: true

                // ARC-UI-2: the shared search input (binds live in
                // `SearchInput`; Ctrl-W word-delete and paste included);
                // Up/Down stay unconsumed context-wise until M4's history.
                SearchInput {
                    id: field
                    anchors.fill: parent
                    flat: true
                    leftPadding: 0
                    rightPadding: 0
                    color: Palette.onSurface
                    font.family: Theme.fonts.body
                    font.pixelSize: bar.height * 0.36

                    onTextChanged: root.refresh()

                    onStepNext: root.step(1)
                    onStepPrevious: root.step(-1)
                    onSubmitted: root.commit()
                    onDismissed: root.abandon()
                }

                // The hint sits where the caret starts, so an empty prompt
                // reads as a command line, not a form field.
                FadeText {
                    anchors.fill: field
                    visible: field.text.length === 0
                    text: "Search this surface"
                    color: Palette.onSurfaceVariant
                    font.family: Theme.fonts.body
                    font.pixelSize: bar.height * 0.36
                    horizontalAlignment: Text.AlignLeft
                    verticalAlignment: Text.AlignVCenter
                    animated: false
                }
            }

            // Neovim's `[i/N]` — matching rows, current / total.
            Text {
                text: root.counterText
                color: Palette.onSurfaceVariant
                font.family: Theme.fonts.body
                font.pixelSize: bar.height * 0.3
                Layout.alignment: Qt.AlignVCenter
                visible: text.length > 0
            }
        }

        // UI-49: the denied cue — a red underline flash on the bar's edge.
        Rectangle {
            id: deniedLine
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: Math.max(1, root.height * Theme.hairlineRatio) * 2
            color: Palette.error
            opacity: 0
        }
    }
}
