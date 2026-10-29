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
import musika
import "Theme.js" as Theme

// ARC-UI-2: the shared search input — the FTS overlay's box and the `/`
// command line — so their editing binds can never drift. Ctrl-W is
// word-delete, paste/copy/select-all stay the TextInput's own; the context
// keys surface as signals for the host to route. Ctrl-H / Ctrl-L mirror
// Shift-Tab / Tab, so the host can change scope without leaving insert mode.
// `flat` drops the chrome for the `/` bar (the bar itself is the surface
// there).
TextField {
    id: root

    /// No background chrome at all (the `/` command line).
    property bool flat: false

    /// Tab / Shift-Tab.
    signal stepNext()
    signal stepPrevious()
    /// Up / Down (and Ctrl-P / Ctrl-N).
    signal stepUp()
    signal stepDown()
    /// Enter.
    signal submitted()
    /// Esc.
    signal dismissed()

    background: Rectangle {
        visible: !root.flat
        color: Palette.surfaceContainerHighest
        radius: Theme.radius
        border.color: root.activeFocus ? Palette.primary : Palette.outlineVariant
        border.width: Theme.hairline(root.height)
    }

    Keys.onPressed: event => {
        if (event.key === Qt.Key_W && (event.modifiers & Qt.ControlModifier)) {
            root.deleteWordBack();
            event.accepted = true;
        } else if (event.key === Qt.Key_H && (event.modifiers & Qt.ControlModifier)) {
            root.stepPrevious();
            event.accepted = true;
        } else if (event.key === Qt.Key_L && (event.modifiers & Qt.ControlModifier)) {
            root.stepNext();
            event.accepted = true;
        } else if (event.key === Qt.Key_Backspace && (event.modifiers & Qt.ControlModifier)) {
            // X11/Qt commonly reports Ctrl-H as Backspace; keep it out of
            // the text field so the scope motion wins.
            root.stepPrevious();
            event.accepted = true;
        } else if (event.key === Qt.Key_Tab) {
            root.stepNext();
            event.accepted = true;
        } else if (event.key === Qt.Key_Backtab) {
            root.stepPrevious();
            event.accepted = true;
        } else if (event.key === Qt.Key_N && (event.modifiers & Qt.ControlModifier)) {
            root.stepDown();
            event.accepted = true;
        } else if (event.key === Qt.Key_P && (event.modifiers & Qt.ControlModifier)) {
            root.stepUp();
            event.accepted = true;
        }
    }
    Keys.onUpPressed: root.stepUp()
    Keys.onDownPressed: root.stepDown()
    Keys.onReturnPressed: root.submitted()
    Keys.onEnterPressed: root.submitted()
    Keys.onEscapePressed: event => {
        root.dismissed();
        event.accepted = true;
    }

    /// Ctrl-W: drop the trailing whitespace run and the word before the
    /// cursor (the FTS field's original bind, moved verbatim).
    function deleteWordBack() {
        var text = root.text;
        var cursor = root.cursorPosition;
        var start = cursor;
        while (start > 0 && /\s/.test(text.charAt(start - 1)))
            start--;
        while (start > 0 && !/\s/.test(text.charAt(start - 1)))
            start--;
        if (start === cursor)
            return;
        root.text = text.slice(0, start) + text.slice(cursor);
        root.cursorPosition = start;
    }
}
