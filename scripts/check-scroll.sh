#!/usr/bin/env sh
# Musika — a keyboard-driven music player.
# Copyright (C) 2026 rPlakama <rPlakama@proton.me>
#
# This program is free software; you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation; either version 2 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program; if not, see <https://www.gnu.org/licenses/>.

# ARC-UI-7.13: headless scroll/search smoke. Drives the real app over the
# IPC socket (search, page/selection moves, fullscreen queue) and fails on
# the runtime warnings `qmllint` cannot see: binding loops, incubation
# destruction, and JS Reference/Type errors.
#
#   nix develop --command cargo build
#   sh scripts/check-scroll.sh
#
# MUSIKA_BIN overrides the binary (default target/debug/musika).
set -eu

cd "$(dirname "$0")/.."

bin=${MUSIKA_BIN:-target/debug/musika}
if [ ! -x "$bin" ]; then
    echo "check-scroll: $bin is missing; build the app first" >&2
    exit 1
fi

sock=$(mktemp -u /tmp/musika-scroll-XXXXXX.sock)
log=$(mktemp)
pid=
cleanup() {
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    rm -f "$sock" "$log"
}
trap cleanup EXIT INT TERM

MUSIKA_IPC_SOCKET=$sock QT_QPA_PLATFORM=offscreen "$bin" >"$log" 2>&1 &
pid=$!

# The socket is created before the accept loop starts; retry until a command
# is actually delivered.
i=0
while [ "$i" -lt 60 ]; do
    if printf 'search\n' | nc -U -w 1 "$sock" 2>/dev/null; then
        break
    fi
    sleep 0.2
    i=$((i + 1))
done

sleep 0.5
for cmd in search_action page_down extend_down escape page_up bottom top \
        layout_toggle move_down move_up layout_toggle \
        toggle_fullscreen page_down bottom top queue_toggle search; do
    printf '%s\n' "$cmd" | nc -U -w 1 "$sock" 2>/dev/null || true
    sleep 0.2
done
sleep 1

if grep -E 'Binding loop detected|Object or context destroyed during incubation|ReferenceError|TypeError' "$log"; then
    echo "check-scroll: QML runtime warning found" >&2
    exit 1
fi

# The 100k-row sparse-snapshot guard needs the QML runtime, which the
# packaged app does not ship on PATH.
if command -v qml >/dev/null 2>&1; then
    if ! QT_QPA_PLATFORM=offscreen qml scripts/bench-searchcontract.qml; then
        echo "check-scroll: SearchContract large-model guard failed" >&2
        exit 1
    fi
fi

echo "check-scroll: ok"
