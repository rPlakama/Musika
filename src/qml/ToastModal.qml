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

// Informational log banner (VI-8a): the neutral sibling of `Toast`, fixed to
// the notice style. Hosts use it for transient logs that are not errors —
// queue yank/paste, future background notices — while `Toast` keeps the
// error/sync/retry surface. Everything else (sizing, wrapping, scrolling,
// fade) is inherited.
Toast {
    id: root

    kind: "log"

    /// Show one log line. Message text is passed through verbatim; the
    /// host's `noticeRaised` channel adds no prefix.
    function log(message) {
        root.show("log", message);
    }
}
