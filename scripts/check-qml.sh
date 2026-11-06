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

# QML verifier, run before building (wired into flake.nix's preBuild) and by
# hand:
#
#   scripts/check-qml.sh
#
# `qmlformat` only parses; it cannot know whether a type exists. `qmllint`
# resolves the QtQuick modules, so a typo like `Alpha` (not a QtQuick.Particles
# type) fails here instead of at `cargo run`'s engine load. The `musika` module
# is registered from Rust at runtime, so its type information is generated for
# tooling into `scripts/lint/musika` (see `scripts/gen-musika-qmltypes.sh`):
# `Palette` is fully typed so a misspelled role fails here; `Backend` is a
# declared singleton without members, so its dynamic surface stays filtered
# below like `QQuickItem`/`QObject`. `.qmllint.ini` records the project's
# warning levels (notably `UnusedImports=warning`).
set -eu

cd "$(dirname "$0")/.."

# The build sandbox does not put the merged Qt env on PATH; callers may point
# QMLLINT at its absolute path (the dev shell keeps the bare command).
qmllint_bin=${QMLLINT:-qmllint}
if ! command -v "$qmllint_bin" >/dev/null 2>&1; then
    echo "check-qml: qmllint is not on PATH" >&2
    exit 1
fi

# The merged Qt env ships qmllint and the QML modules side by side; derive
# the import root from the tool when the shell has not exported it.
if [ -z "${QML2_IMPORT_PATH:-}" ]; then
    qml_root=$(dirname "$(command -v "$qmllint_bin")")/../lib/qt-6/qml
    [ -d "$qml_root" ] && QML2_IMPORT_PATH=$qml_root
fi
export QML2_IMPORT_PATH

log=$(mktemp)
trap 'rm -f "$log"' EXIT

# `-E` reads QML2_IMPORT_PATH (QtQuick, QtQuick.Particles, …); `-I` adds the
# checked-in lint-only `musika` module so Palette/Backend resolve.
"$qmllint_bin" -E -I "$PWD/scripts/lint" src/qml/*.qml >"$log" 2>&1 || true

# The remaining filtered `missing-property` cases are dynamic surfaces: the
# Backend singleton's large slot/property set (not mirrored in the lint
# qmltypes) and the dynamic methods QML reaches through `Item`. Palette roles
# are fully typed now, and every delegate-heavy file carries
# `ComponentBehavior: Bound`, so unqualified access is a hard failure again.
problems=$(grep -E '^(Warning|Error):' "$log" \
    | grep -vE 'not found on type "(QQuickBackend|QQuickItem|QObject)"' \
    | grep -E '\[(import|syntax|compiler|unresolved-type|missing-type|missing-property|incompatible-type|unused-imports)\]|^Error:' || true)

if [ -n "$problems" ]; then
    echo "$problems" >&2
    exit 1
fi

echo "check-qml: ok"
