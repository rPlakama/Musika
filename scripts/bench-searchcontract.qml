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

// ARC-UI-7.13: 100k-row guard for the sparse SearchContract snapshot.
// Run with `qml scripts/bench-searchcontract.qml` (offscreen in CI).
import QtQuick
import "../src/qml" as App

Item {
    width: 800
    height: 600

    property int total: 100000

    ListModel {
        id: songs
    }

    ListView {
        id: view
        anchors.fill: parent
        model: songs
        delegate: Item {
            width: view.width
            height: 20
            required property string title
            Text { text: title }
        }
    }

    App.SearchContract {
        id: contract
        model: songs
        view: view
        fields: ["title"]
    }

    Timer {
        interval: 50
        running: true
        onTriggered: {
            for (var i = 0; i < total; ++i)
                songs.append({ title: "Song " + i });
            Qt.callLater(run);
        }
    }

    function run() {
        var started = Date.now();
        var snapshot = contract.rows();
        var elapsed = Date.now() - started;
        var visible = 0;
        for (var key in snapshot) {
            if (snapshot[key] !== undefined)
                visible++;
        }
        console.log("SearchContract: " + snapshot.length + " rows, "
            + visible + " visible cells, " + elapsed + " ms");
        // Sparse: the full row count is present, only the band carries cells,
        // and the snapshot stays well below a frame.
        if (snapshot.length !== total || visible <= 0 || visible >= total)
            Qt.exit(1);
        if (elapsed > 100)
            Qt.exit(2);
        Qt.exit(0);
    }
}
