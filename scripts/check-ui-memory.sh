#!/usr/bin/env bash
# Sandbox-only UI memory harness for Musika.
#
# Opens N distinct album pages through the real IPC command socket and samples
# the UI process RSS after every entry, then does a revisit phase. Used to prove
# cover-image memory growth (and a plateau once an image cache is bounded).
#
# Usage:
#   scripts/check-ui-memory.sh <musika-bin-dir> [albums=20] [outdir]
#
#   <musika-bin-dir>  directory holding `musika` (and `musika-ui`)
#   [albums]          number of distinct albums to open (default 20)
#   [outdir]          working/output dir (default /tmp/musika-mem/run-<stamp>);
#                     holds config copy, cache, logs, memory.csv
#
# SAFETY: this script NEVER uses the user's real config. It copies
# $MUSIKA_MEM_CONFIG_SRC (default /tmp/musika-mem/config-src, itself a copy of
# ~/.config/musika) into <outdir>/config and points XDG_CONFIG_HOME at it, so
# every database, cover, socket and lock is sandboxed. It only ever kills the
# daemon/UI it spawned itself. Requires a prepared config snapshot; see
#   mkdir -p /tmp/musika-mem && cp -a ~/.config/musika /tmp/musika-mem/config-src
#
# Env overrides:
#   MUSIKA_MEM_CONFIG_SRC   pristine config snapshot (must exist, never the real one)
#   MUSIKA_MEM_SETTLE       seconds to wait after open_song_album (default 1.2)
#   MUSIKA_MEM_INIT         max seconds to wait for the first rendered frame (default 90)
#   MUSIKA_MEM_REVISITS     revisit entries appended (default 10)
#   MUSIKA_MEM_NO_COVERS    1 to set MUSIKA_UI_NO_COVERS=1 for the UI child
#                           (placeholder-only run: isolates cover cost)
set -euo pipefail
export LC_ALL=C  # stable decimal points for awk/printf

die() { echo "check-ui-memory: $*" >&2; exit 1; }

[[ $# -ge 1 ]] || { echo "usage: $0 <musika-bin-dir> [albums] [outdir]" >&2; exit 2; }

BIN_DIR=$(cd "$1" 2>/dev/null && pwd) || die "bin dir not found: $1"
ALBUMS=${2:-20}
[[ "$ALBUMS" =~ ^[0-9]+$ ]] || die "albums must be a number: $ALBUMS"
(( ALBUMS > 0 )) || die "albums must be > 0"
STAMP=$(date +%Y%m%d-%H%M%S)
OUTDIR=${3:-/tmp/musika-mem/run-$STAMP}
mkdir -p "$OUTDIR"
OUTDIR=$(cd "$OUTDIR" && pwd)

[[ -x "$BIN_DIR/musika" ]] || die "missing executable: $BIN_DIR/musika"
[[ -x "$BIN_DIR/musika-ui" ]] || die "missing executable: $BIN_DIR/musika-ui"

SRC=${MUSIKA_MEM_CONFIG_SRC:-/tmp/musika-mem/config-src}
[[ -f "$SRC/library.db" ]] || die "config snapshot not found: $SRC/library.db (set MUSIKA_MEM_CONFIG_SRC)"
SRC_ABS=$(cd "$SRC" && pwd -P)
REAL_CFG="${XDG_CONFIG_HOME:-$HOME/.config/musika}"
case "$SRC_ABS/" in
    "$HOME/.config/musika/"*) die "refusing to run against the user's real config ($SRC_ABS)" ;;
esac
case "$(cd "$OUTDIR" && pwd -P)/" in
    "$HOME/.config/"*) die "outdir must not live under ~/.config ($OUTDIR)" ;;
esac

SETTLE=${MUSIKA_MEM_SETTLE:-1.2}
INIT=${MUSIKA_MEM_INIT:-90}
REVISITS=${MUSIKA_MEM_REVISITS:-10}
NO_COVERS=${MUSIKA_MEM_NO_COVERS:-0}
[[ "$NO_COVERS" =~ ^[01]$ ]] || die "MUSIKA_MEM_NO_COVERS must be 0 or 1"

CFG="$OUTDIR/config"
CACHE="$OUTDIR/cache"
CSV="$OUTDIR/memory.csv"
DAEMON_LOG="$OUTDIR/daemon.log"
UI_LOG="$OUTDIR/ui.log"

DAEMON_PID=""
UI_PID=""

# Send one command line to the sandboxed IPC socket. Prefer a unix-socket
# python one-shot (never blocks on half-close); fall back to nc -U.
send_cmd() {
    local line=$1
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$CFG/ipc.sock" "$line" <<'PY' >/dev/null 2>&1 || true
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(3)
s.connect(sys.argv[1])
s.sendall((sys.argv[2] + "\n").encode())
s.close()
PY
    elif command -v nc >/dev/null 2>&1; then
        printf '%s\n' "$line" | timeout 3 nc -U -N "$CFG/ipc.sock" >/dev/null 2>&1 || true
    else
        die "need python3 or nc to talk to the IPC socket"
    fi
}

vmrss_kib() { awk '/^VmRSS:/{print $2}' "/proc/$1/status" 2>/dev/null || true; }
nthreads() { awk '/^Threads:/{print $2}' "/proc/$1/status" 2>/dev/null || true; }
frame_lines() { grep -c '^\[musika/frame\]' "$UI_LOG" 2>/dev/null || true; }
# Last numeric value of `field=` from the frame log (empty if absent).
last_frame_field() {
    local field=$1
    grep '^\[musika/frame\]' "$UI_LOG" 2>/dev/null \
        | tail -1 | sed -n "s/.* ${field}=\([0-9][0-9.]*\).*/\1/p" || true
}

# CSV is comma-separated, so embedded commas/newlines in user data must go.
sanitize() { tr ',\n\r' '   ' <<<"${1-}"; }

cleanup() {
    set +e
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        send_cmd quit
        # quit flushes and removes the sockets on its own; nudge if it lingers.
        for _ in $(seq 1 20); do kill -0 "$DAEMON_PID" 2>/dev/null || break; sleep 0.1; done
    fi
    [[ -n "$UI_PID" ]] && kill "$UI_PID" 2>/dev/null
    [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null
    [[ -n "$UI_PID" ]] && wait "$UI_PID" 2>/dev/null
    [[ -n "$DAEMON_PID" ]] && wait "$DAEMON_PID" 2>/dev/null
    rm -f "$CFG/ipc.sock" "$CFG/daemon.sock"
    set -e
}
trap cleanup EXIT INT TERM

echo "check-ui-memory: sandbox $OUTDIR"
echo "check-ui-memory: binaries $BIN_DIR"
echo "check-ui-memory: config snapshot $SRC_ABS -> $CFG"

# Fresh per-run config copy (CoW when the filesystem supports it).
rm -rf "$CFG" "$CACHE"
mkdir -p "$CFG" "$CACHE"
cp -a --reflink=auto "$SRC_ABS/." "$CFG/"

# Distinct album representative songs, best effort: one row per album id.
query_songs() {
    local db="$CFG/library.db"
    if command -v python3 >/dev/null 2>&1; then
        python3 - "$db" "$ALBUMS" <<'PY'
import sqlite3, sys
db, limit = sys.argv[1], int(sys.argv[2])
c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
rows = c.execute(
    """
    SELECT s.id, s.album_id, COALESCE(s.album, ''), COALESCE(s.artist, '')
    FROM songs s
    JOIN (
        SELECT album_id, MIN(track) AS t, MIN(id) AS sid
        FROM songs
        WHERE album_id IS NOT NULL AND album_id != ''
        GROUP BY album_id
    ) pick ON s.album_id = pick.album_id AND s.id = pick.sid
    ORDER BY s.album COLLATE NOCASE
    LIMIT ?
    """,
    (limit,),
).fetchall()
for r in rows:
    print("\t".join(str(x) for x in r))
PY
    elif command -v sqlite3 >/dev/null 2>&1; then
        sqlite3 -separator $'\t' "$db" "
        SELECT s.id, s.album_id, COALESCE(s.album,''), COALESCE(s.artist,'')
        FROM songs s JOIN (
            SELECT album_id, MIN(id) AS sid FROM songs
            WHERE album_id IS NOT NULL AND album_id != '' GROUP BY album_id
        ) pick ON s.album_id = pick.album_id AND s.id = pick.sid
        ORDER BY s.album COLLATE NOCASE LIMIT $ALBUMS;"
    else
        die "need python3 or sqlite3 to read $db"
    fi
}

mapfile -t SONG_ROWS < <(query_songs)
(( ${#SONG_ROWS[@]} > 0 )) || die "could not read any song ids from $CFG/library.db"
TOTAL=${#SONG_ROWS[@]}
(( TOTAL >= ALBUMS )) || echo "check-ui-memory: only $TOTAL albums available (asked $ALBUMS)"

# Daemon first: it creates ipc.sock + daemon.sock inside the sandbox config.
echo "check-ui-memory: starting core daemon"
env -u MUSIKA_IPC_SOCKET -u MUSIKA_DAEMON_SOCKET \
    XDG_CONFIG_HOME="$CFG" XDG_CACHE_HOME="$CACHE" \
    "$BIN_DIR/musika" --daemon >"$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

for _ in $(seq 1 300); do
    [[ -S "$CFG/ipc.sock" && -S "$CFG/daemon.sock" ]] && break
    kill -0 "$DAEMON_PID" 2>/dev/null || { tail -20 "$DAEMON_LOG" >&2; die "daemon exited before creating sockets"; }
    sleep 0.1
done
[[ -S "$CFG/ipc.sock" ]] || { tail -20 "$DAEMON_LOG" >&2; die "daemon did not create ipc.sock"; }

# Disposable GPUI client; the core spawns the same `musika-ui` binary.
echo "check-ui-memory: starting UI"
env -u MUSIKA_IPC_SOCKET -u MUSIKA_DAEMON_SOCKET \
    XDG_CONFIG_HOME="$CFG" XDG_CACHE_HOME="$CACHE" MUSIKA_FRAME_STATS=1 \
    MUSIKA_UI_NO_COVERS="$NO_COVERS" \
    "$BIN_DIR/musika-ui" >"$UI_LOG" 2>&1 &
UI_PID=$!

deadline=$((SECONDS + INIT))
while (( SECONDS < deadline )); do
    kill -0 "$UI_PID" 2>/dev/null || { tail -30 "$UI_LOG" >&2; die "UI exited during startup"; }
    [[ "$(frame_lines)" -gt 0 ]] && break
    sleep 0.2
done
[[ "$(frame_lines)" -gt 0 ]] || { tail -30 "$UI_LOG" >&2; die "UI produced no [musika/frame] lines within ${INIT}s"; }

# Let the library settle (initial sync/cover pulls may still be running).
sleep 3
[[ -n "$(vmrss_kib "$UI_PID")" ]] || die "UI process vanished before first sample"

echo "phase,step,album_index,song_id,album,artist,rss_kib,rss_mib,delta_mib,threads,frame_rss_mib,page_rows,frame_lines,img_entries,img_mib" >"$CSV"

sample() { # phase step index song album artist
    local rss threads fr frss prows flines delta
    local song album artist
    song=$(sanitize "${4-}"); album=$(sanitize "${5-}"); artist=$(sanitize "${6-}")
    rss=$(vmrss_kib "$UI_PID")
    [[ -n "$rss" ]] || die "UI process died (phase $1 step $2); see $UI_LOG"
    threads=$(nthreads "$UI_PID")
    frss=$(last_frame_field rss)
    prows=$(last_frame_field page_rows)
    flines=$(frame_lines)
    # Optional (fixed build): bounded image-cache stats from the frame log.
    img_entries=$(last_frame_field covers)
    img_mib=$(last_frame_field covers_mib)
    delta=0
    if [[ -n "${PREV_RSS:-}" ]]; then delta=$(( rss - PREV_RSS )); fi
    PREV_RSS=$rss
    printf '%s,%s,%s,%s,%s,%s,%s,%.1f,%.1f,%s,%s,%s,%s,%s,%s\n' \
        "$1" "$2" "$3" "$song" "$album" "$artist" "$rss" \
        "$(awk -v k="$rss" 'BEGIN{printf "%.1f", k/1024}')" \
        "$(awk -v k="$delta" 'BEGIN{printf "%.1f", k/1024}')" \
        "$threads" "${frss:-}" "${prows:-}" "$flines" "${img_entries:-}" "${img_mib:-}" >>"$CSV"
}

sample baseline 0 0 "" "" ""

echo "check-ui-memory: opening $TOTAL album pages"
i=0
for row in "${SONG_ROWS[@]}"; do
    i=$((i + 1))
    IFS=$'\t' read -r song album_id album artist <<<"$row"
    send_cmd "open_song_album $song"
    sleep "$SETTLE"
    sample entry "$i" "$i" "$song" "$album" "$artist"
    if (( i % 5 == 0 )); then
        echo "check-ui-memory:   $i/$TOTAL entries, RSS $(($(vmrss_kib "$UI_PID") / 1024)) MiB"
    fi
done

if (( REVISITS > 0 )); then
    echo "check-ui-memory: revisit phase ($REVISITS)"
    for ((k = 1; k <= REVISITS; k++)); do
        idx=$(( (k - 1) % TOTAL ))
        IFS=$'\t' read -r song album_id album artist <<<"${SONG_ROWS[$idx]}"
        send_cmd "back"
        sleep "$SETTLE"
        send_cmd "open_song_album $song"
        sleep "$SETTLE"
        sample revisit "$k" "$((idx + 1))" "$song" "$album" "$artist"
    done
fi

# Leave phase: navigate out of the page and confirm the page covers unload.
LEAVE=${MUSIKA_MEM_LEAVE:-1}
LEAVE_WAIT=${MUSIKA_MEM_LEAVE_WAIT:-1.5}
# Optional full-resolution hero probe: open this song's album, sample the cache
# while the hero is painted, go home, sample again. Set MUSIKA_MEM_HERO_SONG to a
# song id whose album has a very large original cover.
HERO_SONG=${MUSIKA_MEM_HERO_SONG:-}
HERO_ALBUM=${MUSIKA_MEM_HERO_ALBUM:-hero}
HERO_REPEAT=${MUSIKA_MEM_HERO_REPEAT:-1}
if (( LEAVE > 0 )); then
    echo "check-ui-memory: leave phase (home, wait ${LEAVE_WAIT}s)"
    before_lines=$(frame_lines)
    # One `home` and no further commands: the render root now requests the
    # extra frames itself, so the departed page's covers age out while idle.
    send_cmd home
    sleep "$LEAVE_WAIT"
    # Wait (passively, no commands) for a fresh [musika/frame] line so
    # covers_mib is not a stale pre-leave value. RSS is the primary signal.
    LEAVE_NEW=0
    deadline=$((SECONDS + 10))
    while (( SECONDS < deadline )); do
        LEAVE_NEW=$(( $(frame_lines) - before_lines ))
        (( LEAVE_NEW > 0 )) && break
        sleep 0.1
    done
    sample leave 1 0 "" "" ""
    # Second, later sample: does RSS settle once the page is gone?
    LEAVE_SETTLE=${MUSIKA_MEM_LEAVE_SETTLE:-6}
    if (( LEAVE_SETTLE > 0 )); then
        sleep "$LEAVE_SETTLE"
        before_lines=$(frame_lines)
        deadline=$((SECONDS + 10))
        while (( SECONDS < deadline )); do
            [[ "$(frame_lines)" -gt "$before_lines" ]] && break
            sleep 0.2
        done
        sample leave_settled 2 0 "" "" ""
    fi
fi

# Full-resolution hero probe: a page whose original cover is huge should show a
# large covers_mib while open and drop back to the row-thumbnail working set
# after `home`, without a lasting RSS step.
if [[ -n "$HERO_SONG" ]]; then
    echo "check-ui-memory: hero probe ($HERO_SONG / $HERO_ALBUM x $HERO_REPEAT)"
    for ((hk = 1; hk <= HERO_REPEAT; hk++)); do
    before_lines=$(frame_lines)
    send_cmd "open_song_album $HERO_SONG"
    sleep "$SETTLE"
    # Wait passively for a fresh [musika/frame] line so covers_mib is not stale.
    HERO_NEW=0
    deadline=$((SECONDS + 15))
    while (( SECONDS < deadline )); do
        HERO_NEW=$(( $(frame_lines) - before_lines ))
        (( HERO_NEW > 0 )) && break
        sleep 0.1
    done
    sample hero "$hk" 0 "$HERO_SONG" "$HERO_ALBUM" ""
    hero_open_rss=$(vmrss_kib "$UI_PID")

    before_lines=$(frame_lines)
    send_cmd home
    sleep "$LEAVE_WAIT"
    HERO_LEAVE_NEW=0
    deadline=$((SECONDS + 15))
    while (( SECONDS < deadline )); do
        HERO_LEAVE_NEW=$(( $(frame_lines) - before_lines ))
        (( HERO_LEAVE_NEW > 0 )) && break
        sleep 0.1
    done
    sample hero_leave "$hk" 0 "$HERO_SONG" "$HERO_ALBUM" ""
    # Later sample: is a very large (>budget) original's memory returned?
    HERO_SETTLE=${MUSIKA_MEM_HERO_SETTLE:-6}
    if (( HERO_SETTLE > 0 )); then
        sleep "$HERO_SETTLE"
        sample hero_settled "$hk" 0 "$HERO_SONG" "$HERO_ALBUM" ""
    fi
    done
fi

echo "check-ui-memory: cleanup"
trap - EXIT INT TERM
cleanup

# ---- compact report -------------------------------------------------------
rss_at() { awk -F, -v i="$1" '$1 == "entry" && $2 == i {printf "%d", $7 / 1024; exit}' "$CSV"; }
threads_at() { awk -F, -v i="$1" '$1 == "entry" && $2 == i {print $10; exit}' "$CSV"; }

base_rss=$(awk -F, '$1 == "baseline" {printf "%d", $7 / 1024; exit}' "$CSV")
final_rss=$(awk -F, -v i="$TOTAL" '$1 == "entry" && $2 == i {printf "%d", $7 / 1024; exit}' "$CSV")
final_threads=$(threads_at "$TOTAL")
page_rows=$(awk -F, -v i="$TOTAL" '$1 == "entry" && $2 == i {print $12; exit}' "$CSV")
frames=$(frame_lines)

echo
echo "UI memory across album-page entries"
echo "  binaries : $BIN_DIR"
echo "  csv      : $CSV"
printf '  %-8s %-8s %-8s %-10s %s\n' entries "RSS MiB" delta "MB/entry" "threads"
base_threads=$(awk -F, '$1 == "baseline" {print $10; exit}' "$CSV")
printf '  %-8s %-8s %-8s %-10s %s\n' 0 "$base_rss" - - "$base_threads"
for n in 5 10 20 40; do
    (( n <= TOTAL )) || continue
    r=$(rss_at "$n"); t=$(threads_at "$n")
    [[ -n "$r" ]] || continue
    d=$(( r - base_rss ))
    printf '  %-8s %-8s %-8s %-10s %s\n' "$n" "$r" "+${d}" "$(awk -v d="$d" -v n="$n" 'BEGIN{printf "%.2f", d/n}')" "$t"
done
if (( TOTAL != 5 && TOTAL != 10 && TOTAL != 20 && TOTAL != 40 )); then
    d=$(( final_rss - base_rss ))
    printf '  %-8s %-8s %-8s %-10s %s\n' "$TOTAL" "$final_rss" "+${d}" "$(awk -v d="$d" -v n="$TOTAL" 'BEGIN{printf "%.2f", d/n}')" "$final_threads"
fi
last_rss=$(last_frame_field rss)
img_entries=$(last_frame_field covers)
img_mib=$(last_frame_field covers_mib)
echo "  frame log: $frames lines, last page_rows=${page_rows:-?}, last rss=${last_rss:-?}MiB"
if [[ -n "$img_entries" ]]; then
    echo "  img cache: ${img_entries} entries, ${img_mib:-?} MiB decoded (last frame)"
fi
page_max=$(awk -F, '$1=="entry" && $15 != "" { if (($15 + 0) > m) { m = $15 + 0; n = $2 } } END { if (m > 0) printf "%s MiB at entry %s", m, n }' "$CSV")
if [[ -n "$page_max" ]]; then
    echo "  pages open: peak covers_mib=$page_max (images painted while a page is open)"
fi
leave_rss=$(awk -F, '$1 == "leave" { printf "%d", $7 / 1024; exit }' "$CSV")
if [[ -n "$leave_rss" ]]; then
    leave_img=$(awk -F, '$1 == "leave" { print $15; exit }' "$CSV")
    fdown=$(( final_rss - leave_rss ))
    if (( ${LEAVE_NEW:-0} > 0 )); then
        echo "  leave/home: ${leave_rss} MiB, covers_mib=${leave_img:-?} (page-open ${final_rss} MiB -> ${fdown} MiB freed; baseline ${base_rss} MiB; fresh frame lines: ${LEAVE_NEW})"
    else
        echo "  leave/home: ${leave_rss} MiB, covers_mib=${leave_img:-?} STALE (page-open ${final_rss} MiB -> ${fdown} MiB freed; baseline ${base_rss} MiB; no fresh frame line - RSS is the signal)"
    fi
    settle_rss=$(awk -F, '$1 == "leave_settled" { printf "%d", $7 / 1024; exit }' "$CSV")
    settle_img=$(awk -F, '$1 == "leave_settled" { print $15; exit }' "$CSV")
    if [[ -n "$settle_rss" ]]; then
        echo "  leave+settle: ${settle_rss} MiB, covers_mib=${settle_img:-?} (vs leave ${leave_rss} MiB)"
    fi
fi

hero_rss=$(awk -F, '$1 == "hero" { r = $7 / 1024 } END { printf "%d", r }' "$CSV")
if [[ -n "$hero_rss" ]]; then
    hero_entries=$(awk -F, '$1 == "hero" { v = $14 } END { print v }' "$CSV")
    hero_mib=$(awk -F, '$1 == "hero" { v = $15 } END { print v }' "$CSV")
    hero_rows=$(awk -F, '$1 == "hero" { v = $12 } END { print v }' "$CSV")
    hero_leave_rss=$(awk -F, '$1 == "hero_leave" { r = $7 / 1024 } END { printf "%d", r }' "$CSV")
    hero_leave_entries=$(awk -F, '$1 == "hero_leave" { v = $14 } END { print v }' "$CSV")
    hero_leave_mib=$(awk -F, '$1 == "hero_leave" { v = $15 } END { print v }' "$CSV")
    echo "  hero probe: page open -> covers=${hero_entries:-?}/${hero_mib:-?} MiB, page_rows=${hero_rows:-?}, RSS ${hero_rss} MiB (fresh frame lines: ${HERO_NEW:-0})"
    echo "             after home -> covers=${hero_leave_entries:-?}/${hero_leave_mib:-?} MiB, RSS ${hero_leave_rss:-?} MiB (fresh frame lines: ${HERO_LEAVE_NEW:-0})"
    hero_settled_rss=$(awk -F, '$1 == "hero_settled" { r = $7 / 1024 } END { printf "%d", r }' "$CSV")
    if [[ -n "$hero_settled_rss" ]]; then
        hero_settled_mib=$(awk -F, '$1 == "hero_settled" { v = $15 } END { print v }' "$CSV")
        echo "             +${HERO_SETTLE:-6}s later -> covers_mib=${hero_settled_mib:-?}, RSS ${hero_settled_rss} MiB"
    fi
fi

# Simple plateau probe: growth of the last quarter vs the first quarter.
if (( TOTAL >= 8 )); then
    q=$(( TOTAL / 4 ))
    early=$(awk -F, -v a="$q" -v b="$((q * 2))" '$1=="entry" && $2>=a && $2<=b {s+=$7; n++} END{if(n)printf "%d", s/n/1024}' "$CSV")
    late_a=$(( TOTAL - q + 1 ))
    late=$(awk -F, -v a="$late_a" -v b="$TOTAL" '$1=="entry" && $2>=a && $2<=b {s+=$7; n++} END{if(n)printf "%d", s/n/1024}' "$CSV")
    echo "  growth  : mid avg ${early} MiB -> final-quarter avg ${late} MiB (delta $((late - early)) MiB)"
fi
echo "  logs     : $DAEMON_LOG $UI_LOG"

exit 0
