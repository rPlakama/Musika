// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

import QtQuick

// Count-only row accessor used by the virtual library lists. Only the live
// window's row objects exist here; every other index returns null.
QtObject {
    id: root

    property int count: 0
    property int version: 0
    property var rows: ({})

    function reset(nextCount) {
        root.rows = ({});
        root.count = nextCount;
        root.version++;
    }

    function clearRows() {
        root.rows = ({});
        root.version++;
    }

    function setWindow(start, windowRows) {
        var next = ({});
        for (var i = 0; i < windowRows.length; ++i)
            next[start + i] = windowRows[i];
        root.rows = next;
        root.version++;
    }

    function rowAt(index) {
        return root.rows[index] !== undefined ? root.rows[index] : null;
    }

    function get(index) {
        return root.rowAt(index);
    }

    function setProperty(index, key, value) {
        var row = root.rows[index];
        if (row === undefined || row === null)
            return;
        row[key] = value;
        root.version++;
    }
}
