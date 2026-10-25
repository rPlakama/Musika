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

# ARC-36: read-only modularity inventory. Prints per-file size/function
# counts, cross-module references, the repeated-pattern counts the audit
# watches, and (with `--inventory`) every Rust function signature.
set -eu

cd "$(dirname "$0")/.."
inventory=${1:-}

files=$(find src -name '*.rs' ! -name 'test.rs' | sort)

printf '== modules (loc, functions, pub functions)\n'
printf '%-34s %6s %5s %5s\n' file loc fns pub
for f in $files; do
    loc=$(wc -l <"$f")
    fns=$(grep -Ec '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' "$f" || true)
    pub=$(grep -Ec '^[[:space:]]*pub(\([^)]*\))?[[:space:]]+(async[[:space:]]+)?fn[[:space:]]+' "$f" || true)
    printf '%-34s %6s %5s %5s\n' "$f" "$loc" "$fns" "$pub"
done

printf '\n== totals\n'
printf 'production files: %s\n' "$(printf '%s\n' "$files" | wc -l | tr -d ' ')"
printf 'production loc:   %s\n' "$(wc -l $files | tail -1 | awk '{print $1}')"
printf 'production fns:   %s\n' "$(grep -RhEc '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' $files | awk '{s+=$1} END{print s}')"
printf 'test loc:         %s\n' "$(wc -l <src/test.rs)"
printf 'qslots:           %s\n' "$(grep -c '#\[qslot' src/qml.rs || true)"

printf '\n== cross-module crate:: references\n'
grep -rhoE 'crate::[a-z_]+' src --include='*.rs' --exclude='test.rs' \
    | sed 's/^crate:://' | sort | uniq -c | sort -rn | head -20

printf '\n== repeated patterns\n'
printf 'md5_hex defs:          %s\n' "$(grep -R 'fn md5_hex' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'url_map_of calls:      %s\n' "$(grep -R 'url_map_of(' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'cover_url calls:       %s\n' "$(grep -RE 'cover_url(_ref)?\(' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'chunks(500):           %s\n' "$(grep -R 'chunks(500)' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'qml::{online,client}:  %s\n' "$(grep -R 'crate::qml::\(online\|client\)' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'crate::{dbs,runtime}:  %s\n' "$(grep -R 'crate::\(dbs\|runtime\|pool\)(' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"
printf 'eprintln!:             %s\n' "$(grep -R 'eprintln!' src --include='*.rs' --exclude='test.rs' | wc -l | tr -d ' ')"

if [ "$inventory" = "--inventory" ]; then
    printf '\n== function inventory (file:line:signature)\n'
    grep -RHnE '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+' $files
fi
