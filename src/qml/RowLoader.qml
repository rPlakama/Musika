// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

import QtQuick

// Row payload worker: JSON parsing and field mapping happen off the GUI
// thread, in 128-row batches. This does not create QML items; lists still
// instantiate delegates on the GUI thread.
QtObject {
    id: root

    property int _nextId: 1

    signal batch(int requestId, string kind, var rows, bool done, var meta)
    signal failed(int requestId, string kind, string message)

    function load(kind, payload) {
        var id = root._nextId++;
        worker.sendMessage({ id: id, kind: kind, payload: payload });
        return id;
    }

    property WorkerScript worker: WorkerScript {
        source: "RowWorker.js"
        onMessage: function (message) {
            if (message.error) {
                root.failed(message.id, message.kind, message.error);
                return;
            }
            root.batch(message.id, message.kind, message.rows,
                message.done, message.meta);
        }
    }
}
