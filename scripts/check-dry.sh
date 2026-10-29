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

# DRY guard: the QML tree owns each repeated policy in exactly one file.
# A new copy of one of these recipes fails the build instead of quietly
# drifting from its owner.
#
#   scripts/check-dry.sh
#
# The checks read source lines, so a comment mentioning a recipe is ignored.
set -eu

cd "$(dirname "$0")/.."

fail=0

check() {
    label=$1
    pattern=$2
    allow=$3
    hits=$(grep -RInE "$pattern" src/qml --include='*.qml' \
        | grep -vE ':[0-9]+:[[:space:]]*//' || true)
    if [ -n "$allow" ]; then
        hits=$(printf '%s\n' "$hits" | grep -vE "$allow" || true)
    fi
    if [ -n "$hits" ]; then
        printf 'check-dry: %s has a second owner:\n%s\n' "$label" "$hits" >&2
        fail=1
    fi
}

# One download-mark window engine.
check "download-window requests" \
    'Backend\.(requestDownloads|refreshDownloads)\(' \
    'MarkWindow\.qml:'
check "mark-window implementations" \
    'function requestMarkWindow' \
    '(ScrollContent|ScrollPanel|SearchColumn)\.qml:'

# One motion easing vocabulary.
check "Bezier easing boilerplate" \
    'easing\.type:[[:space:]]*Easing\.Bezier' \
    '(Anim|ColorAnim)\.qml:'

# One hairline clamp.
check "hairline clamp" \
    'Math\.max\(1,[^)]*hairlineRatio' \
    ''

# One lazy-load state machine.
check "preload flags" \
    'property bool _preloaded' \
    'MediaList\.qml:'
check "preload timer" \
    'id: preloadTimer' \
    'MediaList\.qml:'

# One optimistic favourite wrapper.
check "optimistic favourite wrapper" \
    'actions\.south\.invoke[[:space:]]*=' \
    'QuickActions\.qml:'

if [ "$fail" -ne 0 ]; then
    echo "check-dry: duplicated policy found" >&2
    exit 1
fi

echo "check-dry: ok"
