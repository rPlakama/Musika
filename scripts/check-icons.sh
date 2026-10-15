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

# ARC-41: icon pipeline check, run before building (wired into flake.nix's
# preBuild) and by hand:
#
#   scripts/check-icons.sh            # unreferenced assets only warn
#   scripts/check-icons.sh --strict   # unreferenced assets fail too
#
# Rules:
#   1. every glyph asset carries a baked white fill/stroke
#      (run scripts/bake-icons-white.sh otherwise);
#   2. every `qrc:/app/icons/...` reference in `src/` resolves to a file;
#   3. every asset is referenced by the tree or listed as brand below.
set -eu

cd "$(dirname "$0")/.."

strict=0
[ "${1:-}" = "--strict" ] && strict=1

# Multicolor / packaging assets: not glyphs, so exempt from the white bake and
# from the referenced rule (musika.svg is installed by PKG-1, P.svg is its
# brand source, the provider logos are kept for the server badge).
brand="musika.svg SpinlogoMusika.svg P.svg subsonic_94696.svg navidrome.svg"

fail=0
warn=0

# 1. Baked white ink.
for f in src/icons/*.svg; do
    name=${f##*/}
    case " $brand " in *" $name "*) continue ;; esac
    if ! grep -qE '(fill|stroke)[=:]"?#FFFFFF' "$f"; then
        echo "check-icons: $name has no baked white fill/stroke — run scripts/bake-icons-white.sh" >&2
        fail=1
    fi
done

# 2. Every reference resolves.
refs=$(grep -rhoE 'qrc:/app/icons/[A-Za-z0-9._-]+\.svg' src | sort -u)
for ref in $refs; do
    name=${ref#qrc:/app/icons/}
    if [ ! -f "src/icons/$name" ]; then
        echo "check-icons: $ref is referenced but src/icons/$name does not exist" >&2
        fail=1
    fi
done

# 3. Every asset is referenced or brand-listed.
for f in src/icons/*.svg; do
    name=${f##*/}
    case " $brand " in *" $name "*) continue ;; esac
    case "$refs" in
        *"qrc:/app/icons/$name"*) ;;
        *)
            echo "check-icons: $name is never referenced — reference it, brand-list it, or delete it" >&2
            warn=1
            ;;
    esac
done

if [ "$fail" -ne 0 ] || { [ "$strict" -eq 1 ] && [ "$warn" -ne 0 ]; }; then
    exit 1
fi
echo "check-icons: ok"
