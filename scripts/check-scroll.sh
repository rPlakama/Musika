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

# Headless scroll/search smoke. Drives the real app over the IPC socket.
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

MUSIKA_IPC_SOCKET=$sock "$bin" >"$log" 2>&1 &
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


echo "check-scroll: ok"
