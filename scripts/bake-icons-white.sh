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

# Bake SVG glyphs white (#FFFFFF). SymbolIcon re-colours assets through
# MultiEffect, and the D-6 convention keeps a white fill baked in, so a
# partially colourised or uncolourised draw still shows. Handles attribute
# fills/strokes and CSS `.cls { fill:#... }` rules.
#
#   scripts/bake-icons-white.sh                      # the svgrepo glyph set
#   scripts/bake-icons-white.sh src/icons/foo.svg    # explicit files
#
# Brand/provider assets (`musika.svg`, `P.svg`, `subsonic_94696.svg`,
# `navidrome.svg`) are not svgrepo-named and stay untouched unless passed
# explicitly.
set -eu

cd "$(dirname "$0")/.."

if [ "$#" -eq 0 ]; then
    set -- src/icons/*-svgrepo-com.svg
fi

for f in "$@"; do
    sed -i -E \
        -e 's/(fill|stroke|stop-color)="#[0-9A-Fa-f]{3,8}"/\1="#FFFFFF"/g' \
        -e 's/(fill|stroke|stop-color):#[0-9A-Fa-f]{3,8}/\1:#FFFFFF/g' \
        -e 's/(fill|stroke|stop-color)="currentColor"/\1="#FFFFFF"/g' \
        "$f"
    # Assets that carry no colour at all (paths inherit the black default)
    # are baked by setting the fill on the root <svg>; an inherited
    # fill="none" root is switched to white instead of duplicated.
    if ! grep -qE '(fill|stroke)[=:]"?#FFFFFF' "$f"; then
        if grep -qE '<svg[^>]*fill="none"' "$f"; then
            sed -i -E '0,/<svg/{s/fill="none"/fill="#FFFFFF"/}' "$f"
        else
            sed -i -E '0,/<svg/{s/<svg/<svg fill="#FFFFFF"/}' "$f"
        fi
    fi
    printf 'baked: %s\n' "$f"
done
