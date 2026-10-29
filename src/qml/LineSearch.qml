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

// ARC-UI-2 M1/M2: the `/` in-surface search command line — a flat, full-width
// bar anchored to the bottom edge over a 50% black scrim (no blur). The whole
// window is the buffer: every visible row surface answers at once, matching
// is literal smartcase over the rows the user can actually see, and the
// matched substrings paint their hlsearch blocks (SearchText) live. Tab /
// Shift-Tab cycle the matching rows (the cursor previews each in place) and
// Enter commits: the prompt closes and the owning surface's own context-Enter
// runs on the match — open, play, jump, whatever the row's enter action is.
// Esc abandons; a non-empty query with no matches flashes the denied cue and
// never moves the cursor.
Item {
    id: root

    property bool opened: false
    /// The surfaces being searched, in window order — the whole window is
    /// the buffer: each entry is a proxy (`searchRows()`/`searchJump(row)`/
    /// `setQuery(q, row)`) built by Home.
    property var targets: []
    /// Every row's rendered cells, concatenated in target order. Sparse:
    /// only visible rows carry cells (off-band rows are holes), and the
    /// index stays the absolute row so `searchJump` needs no remapping.
    property var _rows: []
    /// Cumulative model row counts per target (offsets into `_rows`).
    property var _offsets: []
    /// Candidate row indices (global), in buffer order.
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

    function open(list) {
        opened = true;
        targets = list || [];
        // The global keymap dies while the command line types (see
        // `Backend::key_event`).
        Backend.setLineSearchOpen(true);
        _rows = [];
        _offsets = [];
        for (var i = 0; i < targets.length; ++i) {
            var rows = targets[i].searchRows();
            var base = _offsets.length > 0 ? _offsets[_offsets.length - 1] : 0;
            for (var r = 0; r < rows.length; ++r) {
                if (rows[r] !== undefined)
                    _rows[base + r] = rows[r];
            }
            _offsets.push(base + rows.length);
        }
        _candidates = [];
        _index = -1;
        _zeroQuery = "";
        field.text = "";
        field.forceActiveFocus();
    }

    function close() {
        for (var i = 0; i < targets.length; ++i)
            targets[i].setQuery("", -1);
        opened = false;
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
        publish();
    }

    function jump() {
        if (_index < 0 || _candidates.length === 0)
            return;
        var g = _candidates[_index];
        var t = 0;
        while (t < _offsets.length && g >= _offsets[t])
            t++;
        if (t >= targets.length)
            return;
        var start = t === 0 ? 0 : _offsets[t - 1];
        targets[t].searchJump(g - start);
    }

    /// Push the session down to every surface: the query everywhere (the row
    /// blocks paint), the candidate row only where it lives (`CurSearch`).
    function publish() {
        var g = (_index >= 0 && _candidates.length > 0) ? _candidates[_index] : -1;
        for (var i = 0; i < targets.length; ++i) {
            var start = i === 0 ? 0 : _offsets[i - 1];
            var here = g >= start && g < _offsets[i];
            targets[i].setQuery(field.text, here ? g - start : -1);
        }
    }

    /// Cycle the candidate ring; Tab forward, Shift-Tab back, wrapping.
    function step(delta) {
        if (_candidates.length === 0)
            return;
        _index = (_index + delta + _candidates.length) % _candidates.length;
        jump();
        publish();
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
        else if (action === "back" || action === "cancel")
            abandon();
    }

    function denied() {
        if (!Theme.reduceMotion)
            deniedFlash.restart();
    }

    visible: opened || bar.opacity > 0
    z: Theme.zCommand

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
    Scrim {
        shown: root.opened
        strength: 0.5
        easingCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate
    }

    // The command line itself: full width, the shared soft radius, no border
    // highlight — one vertically centred line of prompt (Neovim's last row,
    // reskinned).
    Rectangle {
        radius: Theme.radius
        id: bar
        width: root.width
        height: root.height * 0.07
        // Slides up from below the bottom edge on open, and back down on
        // close (decelerate in / accelerate out, motion §3.1).
        y: root.height - height + (root.opened ? 0 : height)
        color: Palette.surface
        opacity: root.opened ? 1 : 0
        clip: true

        Behavior on y { Anim { duration: Theme.motion.standard; easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate } }
        Behavior on opacity { Anim { duration: Theme.motion.standard; easing.bezierCurve: root.opened ? Theme.ease.decelerate : Theme.ease.accelerate } }

        RowLayout {
            anchors.fill: parent
            anchors.leftMargin: root.width * 0.04
            anchors.rightMargin: root.width * 0.04
            spacing: root.height * 0.012

            // The Neovim command-line prefix.
            AppText {
                text: "/"
                color: Palette.primary
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
                    font.pixelSize: bar.height * 0.36
                    horizontalAlignment: Text.AlignLeft
                    verticalAlignment: Text.AlignVCenter
                    animated: false
                }
            }

            // Neovim's `[i/N]` — matching rows, current / total.
            AppText {
                text: root.counterText
                color: Palette.onSurfaceVariant
                font.pixelSize: bar.height * 0.3
                Layout.alignment: Qt.AlignVCenter
                visible: text.length > 0
            }
        }

        // UI-49: the denied cue — a red underline flash on the bar's edge.
        Rectangle {
            radius: Theme.radius
            id: deniedLine
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: Theme.hairline(root.height) * 2
            color: Palette.error
            opacity: 0
        }
    }
}
