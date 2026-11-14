#!/usr/bin/env bash
# Sandbox-only cover-load latency harness for Musika.
#
# Measures the current, user-visible cover path end to end:
#   * core daemon startup: spawn -> both IPC sockets present
#   * UI startup: spawn -> first `[musika/frame]` summary line, and the first
#     `[musika/cover] ready` line on the initial library screen
#   * one representative song per distinct album: send `open_song_album`,
#     then wait for the first `[musika/cover] ready ... id=page-hero` line
#     whose epoch_ms is at or after the send timestamp. The difference is the
#     hero-cover ready latency written to cover-load.csv.
#   * a small revisit phase: reopen one album page and compare it with its
#     first open, which quantifies the decode repeated after a page leave.
#
# The UI must be built with the `MUSIKA_LOAD_TRACE` instrumentation in
# src/ui/widgets/cover_image.rs; without it the run aborts at startup with a
# clear "no [musika/cover] ready lines" error. Newer builds may append
# `state=preview|full` to that line (a cheap placeholder painted while the
# source decodes); this harness times the first ready line, i.e. the first
# pixels painted, and ignores the suffix.
#
# Usage:
#   scripts/check-cover-load.sh <musika-bin-dir> [albums=20] [outdir]
#
#   <musika-bin-dir>  directory holding `musika` and `musika-ui`
#   [albums]          number of distinct albums to open (default 20)
#   [outdir]          working/output dir (default /tmp/musika-load/run-<stamp>);
#                     holds the config copy, cache, logs, cover-load.csv and
#                     summary.txt
#
# SAFETY: this script NEVER uses the user's real config or cache. It copies
# $MUSIKA_LOAD_CONFIG_SRC (default /tmp/musika-mem/config-src, itself a copy of
# ~/.config/musika) into <outdir>/config and points XDG_CONFIG_HOME at it, and
# points XDG_CACHE_HOME at <outdir>/cache, so every database, cover, socket and
# lock is sandboxed. It only ever kills the daemon/UI/sender it spawned itself,
# and refuses to run against the real config. Requires a prepared snapshot:
#   mkdir -p /tmp/musika-mem && cp -a ~/.config/musika /tmp/musika-mem/config-src
#
# Env overrides:
#   MUSIKA_LOAD_CONFIG_SRC   pristine config snapshot (never the real config)
#   MUSIKA_LOAD_HOME_SETTLE  seconds after startup before the album phase (3)
#   MUSIKA_LOAD_SETTLE       seconds after the hero is ready before `back` (0.6)
#   MUSIKA_LOAD_BACK_SETTLE  seconds after `back` before the next open (0.8)
#   MUSIKA_LOAD_READY_GRACE  seconds to collect ready lines after the hero (0.08)
#   MUSIKA_LOAD_HERO_TIMEOUT seconds to wait for a page hero (5)
#   MUSIKA_LOAD_INIT         max seconds to wait for the first frame line (90)
#   MUSIKA_LOAD_FIRST_COVER  max seconds to wait for the first cover line (10)
#   MUSIKA_LOAD_POLL         polling granularity in seconds (0.03)
#   MUSIKA_LOAD_HIT_MS       "fast/cached" hero threshold in ms (50)
#   MUSIKA_LOAD_REVISITS     reopen samples in the revisit phase (1)
#   MUSIKA_LOAD_BOUNCE       immediate reopen pair for the warm-cover check (1)
#   MUSIKA_LOAD_PAUSE        pause the restored session after startup (1)
set -euo pipefail
export LC_ALL=C  # stable decimal points for awk/printf

die() { echo "check-cover-load: $*" >&2; exit 1; }
now_ms() { date +%s%3N; }  # GNU coreutils; the harness is Linux-only

[[ $# -ge 1 ]] || { echo "usage: $0 <musika-bin-dir> [albums] [outdir]" >&2; exit 2; }

BIN_DIR=$(cd "$1" 2>/dev/null && pwd) || die "bin dir not found: $1"
ALBUMS=${2:-20}
[[ "$ALBUMS" =~ ^[0-9]+$ ]] || die "albums must be a number: $ALBUMS"
(( ALBUMS > 0 )) || die "albums must be > 0"
STAMP=$(date +%Y%m%d-%H%M%S)
OUTDIR=${3:-/tmp/musika-load/run-$STAMP}
mkdir -p "$OUTDIR"
OUTDIR=$(cd "$OUTDIR" && pwd)

[[ -x "$BIN_DIR/musika" ]] || die "missing executable: $BIN_DIR/musika"
[[ -x "$BIN_DIR/musika-ui" ]] || die "missing executable: $BIN_DIR/musika-ui"

SRC=${MUSIKA_LOAD_CONFIG_SRC:-/tmp/musika-mem/config-src}
[[ -f "$SRC/library.db" ]] || die "config snapshot not found: $SRC/library.db (set MUSIKA_LOAD_CONFIG_SRC)"
SRC_ABS=$(cd "$SRC" && pwd -P)
REAL_CFG="$HOME/.config/musika"
case "$SRC_ABS/" in
    "$REAL_CFG/"*) die "refusing to run against the user's real config ($SRC_ABS)" ;;
esac
if [[ -n "${XDG_CONFIG_HOME:-}" ]]; then
    XDG_CFG_ABS=$(cd "$XDG_CONFIG_HOME" 2>/dev/null && pwd -P) || XDG_CFG_ABS=""
    if [[ -n "$XDG_CFG_ABS" && "$SRC_ABS" == "$XDG_CFG_ABS" ]]; then
        die "refusing to run against the user's real config ($SRC_ABS)"
    fi
fi
case "$OUTDIR/" in
    "$HOME/.config/"*) die "outdir must not live under ~/.config ($OUTDIR)" ;;
esac

HOME_SETTLE=${MUSIKA_LOAD_HOME_SETTLE:-3}
SETTLE=${MUSIKA_LOAD_SETTLE:-0.6}
BACK_SETTLE=${MUSIKA_LOAD_BACK_SETTLE:-0.8}
READY_GRACE=${MUSIKA_LOAD_READY_GRACE:-0.08}
HERO_TIMEOUT=${MUSIKA_LOAD_HERO_TIMEOUT:-5}
INIT=${MUSIKA_LOAD_INIT:-90}
FIRST_COVER=${MUSIKA_LOAD_FIRST_COVER:-10}
POLL=${MUSIKA_LOAD_POLL:-0.03}
HIT_MS=${MUSIKA_LOAD_HIT_MS:-50}
REVISITS=${MUSIKA_LOAD_REVISITS:-1}
BOUNCE=${MUSIKA_LOAD_BOUNCE:-1}
PAUSE=${MUSIKA_LOAD_PAUSE:-1}
[[ "$HERO_TIMEOUT" =~ ^[0-9]+$ ]] || die "MUSIKA_LOAD_HERO_TIMEOUT must be a whole number of seconds"
[[ "$INIT" =~ ^[0-9]+$ ]] || die "MUSIKA_LOAD_INIT must be a whole number of seconds"
[[ "$FIRST_COVER" =~ ^[0-9]+$ ]] || die "MUSIKA_LOAD_FIRST_COVER must be a whole number of seconds"
[[ "$REVISITS" =~ ^[0-9]+$ ]] || die "MUSIKA_LOAD_REVISITS must be a number"
[[ "$PAUSE" =~ ^[01]$ ]] || die "MUSIKA_LOAD_PAUSE must be 0 or 1"

CFG="$OUTDIR/config"
CACHE="$OUTDIR/cache"
CSV="$OUTDIR/cover-load.csv"
SUMMARY="$OUTDIR/summary.txt"
DAEMON_LOG="$OUTDIR/daemon.log"
UI_LOG="$OUTDIR/ui.log"

DAEMON_PID=""
UI_PID=""
IPC_PID=""
FAILURES=0

# Persistent IPC sender: one python process holds the ipc.sock connection and
# forwards one command line per stdin line. A per-command python one-shot costs
# ~30-40 ms just to start the interpreter, which would pollute the latency
# samples; the coproc write here is well under a millisecond.
start_ipc_sender() {
    coproc IPC_SENDER { python3 -u -c '
import socket, sys

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(3)
try:
    sock.connect(sys.argv[1])
except OSError as exc:
    sys.stderr.write(f"check-cover-load: ipc sender connect failed: {exc}\n")
    raise SystemExit(1)
for line in sys.stdin:
    line = line.rstrip("\n")
    if not line:
        continue
    try:
        sock.sendall((line + "\n").encode())
    except OSError as exc:
        sys.stderr.write(f"check-cover-load: ipc sender failed: {exc}\n")
        break
' "$CFG/ipc.sock"; }
    IPC_PID=$IPC_SENDER_PID
}

send_cmd() {
    local line=$1
    if [[ -z "$IPC_PID" ]] || ! kill -0 "$IPC_PID" 2>/dev/null; then
        tail -20 "$UI_LOG" >&2 2>/dev/null || true
        die "IPC sender is not running (daemon/UI may have exited)"
    fi
    printf '%s\n' "$line" >&"${IPC_SENDER[1]}" || die "failed to queue IPC command: $line"
}

cleanup() {
    set +e
    if [[ -n "$IPC_PID" ]] && kill -0 "$IPC_PID" 2>/dev/null; then
        printf 'quit\n' >&"${IPC_SENDER[1]}" 2>/dev/null
    fi
    if [[ -n "$DAEMON_PID" ]] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        # quit flushes and removes the sockets on its own; nudge if it lingers.
        for _ in $(seq 1 20); do
            kill -0 "$DAEMON_PID" 2>/dev/null || break
            sleep 0.1
        done
    fi
    [[ -n "$UI_PID" ]] && kill "$UI_PID" 2>/dev/null
    [[ -n "$IPC_PID" ]] && kill "$IPC_PID" 2>/dev/null
    [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null
    [[ -n "$UI_PID" ]] && wait "$UI_PID" 2>/dev/null
    [[ -n "$IPC_PID" ]] && wait "$IPC_PID" 2>/dev/null
    [[ -n "$DAEMON_PID" ]] && wait "$DAEMON_PID" 2>/dev/null
    rm -f "$CFG/ipc.sock" "$CFG/daemon.sock"
    set -e
}
trap cleanup EXIT INT TERM

log_size() { wc -c <"$UI_LOG" 2>/dev/null | tr -d ' ' || echo 0; }
frame_lines() { grep -c '^\[musika/frame\]' "$UI_LOG" 2>/dev/null || true; }

# First `[musika/cover] ready` line at/after byte `offset` whose epoch_ms is at
# or after `threshold`; prints the epoch (ms) or nothing. Lines are
# `[musika/cover] ready id=<id> index=<n> epoch_ms=<ms> edge=<px>`.
first_hero_epoch() {
    local offset=$1 threshold=$2
    tail -c +$((offset + 1)) "$UI_LOG" 2>/dev/null | awk -v t="$threshold" '
        $1 == "[musika/cover]" && $2 == "ready" && $3 == "id=page-hero" {
            epoch = substr($5, 10) + 0
            if (epoch >= t) { print epoch; exit }
        }'
}

# Count ready lines since `offset`: prints "<all-covers> <page-hero>".
count_ready() {
    local offset=$1
    tail -c +$((offset + 1)) "$UI_LOG" 2>/dev/null | awk '
        $1 == "[musika/cover]" && $2 == "ready" {
            covers++
            if ($3 == "id=page-hero") hero++
        }
        END { printf "%d %d", covers + 0, hero + 0 }'
}

# CSV is comma-separated, so embedded commas/newlines in user data must go.
sanitize() { tr ',\n\r' '   ' <<<"${1-}"; }

echo "check-cover-load: sandbox $OUTDIR"
echo "check-cover-load: binaries $BIN_DIR"
echo "check-cover-load: config snapshot $SRC_ABS -> $CFG"

# Fresh per-run config copy (CoW when the filesystem supports it).
rm -rf "$CFG" "$CACHE"
mkdir -p "$CFG" "$CACHE"
cp -a --reflink=auto "$SRC_ABS/." "$CFG/"

# Distinct album representative songs, best effort: one row per album id.
query_songs() {
    local db="$CFG/library.db"
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
}

mapfile -t SONG_ROWS < <(query_songs)
(( ${#SONG_ROWS[@]} > 0 )) || die "could not read any song ids from $CFG/library.db"
TOTAL=${#SONG_ROWS[@]}
(( TOTAL >= ALBUMS )) || echo "check-cover-load: only $TOTAL albums available (asked $ALBUMS)"

# Daemon first: it creates ipc.sock + daemon.sock inside the sandbox config.
echo "check-cover-load: starting core daemon"
daemon_spawn_ms=$(now_ms)
env -u MUSIKA_IPC_SOCKET -u MUSIKA_DAEMON_SOCKET \
    XDG_CONFIG_HOME="$CFG" XDG_CACHE_HOME="$CACHE" \
    "$BIN_DIR/musika" --daemon >"$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

deadline=$((daemon_spawn_ms + 30000))
while (( $(now_ms) < deadline )); do
    [[ -S "$CFG/ipc.sock" && -S "$CFG/daemon.sock" ]] && break
    kill -0 "$DAEMON_PID" 2>/dev/null || { tail -20 "$DAEMON_LOG" >&2; die "daemon exited before creating sockets"; }
    sleep 0.05
done
[[ -S "$CFG/ipc.sock" && -S "$CFG/daemon.sock" ]] || { tail -20 "$DAEMON_LOG" >&2; die "daemon did not create both sockets within 30s"; }
daemon_ready_ms=$(( $(now_ms) - daemon_spawn_ms ))

start_ipc_sender
for _ in $(seq 1 20); do
    kill -0 "$IPC_PID" 2>/dev/null && break
    sleep 0.1
done
kill -0 "$IPC_PID" 2>/dev/null || die "persistent IPC sender failed to connect to $CFG/ipc.sock"

# Disposable GPUI client; the core spawns the same `musika-ui` binary.
echo "check-cover-load: starting UI"
ui_spawn_ms=$(now_ms)
env -u MUSIKA_IPC_SOCKET -u MUSIKA_DAEMON_SOCKET \
    XDG_CONFIG_HOME="$CFG" XDG_CACHE_HOME="$CACHE" \
    MUSIKA_FRAME_STATS=1 MUSIKA_LOAD_TRACE=1 \
    "$BIN_DIR/musika-ui" >"$UI_LOG" 2>&1 &
UI_PID=$!

deadline=$((ui_spawn_ms + INIT * 1000))
ui_first_frame_ms=""
while (( $(now_ms) < deadline )); do
    kill -0 "$UI_PID" 2>/dev/null || { tail -30 "$UI_LOG" >&2; die "UI exited during startup"; }
    if grep -q '^\[musika/frame\]' "$UI_LOG" 2>/dev/null; then
        ui_first_frame_ms=$(( $(now_ms) - ui_spawn_ms ))
        break
    fi
    sleep "$POLL"
done
[[ -n "$ui_first_frame_ms" ]] || { tail -30 "$UI_LOG" >&2; die "UI produced no [musika/frame] lines within ${INIT}s"; }

# First cover line on the initial screen (library rows / player bar). It is
# normally emitted well before the 120-frame summary above.
first_cover_line=""
deadline=$(( $(now_ms) + FIRST_COVER * 1000 ))
while (( $(now_ms) < deadline )); do
    first_cover_line=$(grep -m1 '^\[musika/cover\] ready ' "$UI_LOG" 2>/dev/null || true)
    [[ -n "$first_cover_line" ]] && break
    kill -0 "$UI_PID" 2>/dev/null || { tail -30 "$UI_LOG" >&2; die "UI exited while waiting for the first cover"; }
    sleep 0.05
done
[[ -n "$first_cover_line" ]] || {
    tail -30 "$UI_LOG" >&2
    die "no [musika/cover] ready lines within ${FIRST_COVER}s; is the binary built with the MUSIKA_LOAD_TRACE instrumentation?"
}
first_cover_epoch=$(awk '{ print substr($5, 10) + 0 }' <<<"$first_cover_line")
first_cover_id=$(awk '{ print substr($3, 4) }' <<<"$first_cover_line")
first_cover_index=$(awk '{ print substr($4, 7) }' <<<"$first_cover_line")
first_cover_edge=$(awk '{ print substr($6, 6) }' <<<"$first_cover_line")
first_home_cover_ms=$(( first_cover_epoch - ui_spawn_ms ))

# Raise/activate the window through the same path as `musika show`. A window
# the compositor considers inactive is throttled (this host rendered it only
# every ~2.5 s), which would turn hero readiness into frame-scheduling latency
# instead of cover-decode latency. The first cover line above proves our UI is
# attached, so the core broadcasts ACTIVATE instead of spawning a second UI.
send_cmd show
sleep 0.5

# The snapshot restored a playing session, which keeps the UI painting at
# ~60 fps and decodes audio for the whole run. Once startup has been measured,
# pause through the real command path so the album phase measures cover work
# rather than audio/render churn. MUSIKA_LOAD_PAUSE=0 leaves playback restored.
if (( PAUSE )); then
    send_cmd pause
fi

# Let the library settle (initial sync/cover pulls may still be running).
sleep "$HOME_SETTLE"
[[ -n "$(awk '/^VmRSS:/{print $2}' "/proc/$UI_PID/status" 2>/dev/null || true)" ]] || die "UI process vanished before the album phase"

# The core replays the last open page on connect. Return to the library so the
# first measured album is opened from a known home state (and an in-flight
# restored hero cannot be mistaken for album 1's).
send_cmd back
sleep "$BACK_SETTLE"

echo "check-cover-load: startup: daemon ${daemon_ready_ms}ms, first frame ${ui_first_frame_ms}ms, first cover ${first_home_cover_ms}ms (${first_cover_id})"

echo "phase,album_index,song_id,album,artist,send_ms,send_done_ms,send_overhead_ms,hero_ready_epoch_ms,hero_ready_ms,hero_ready_lines,cover_ready_lines,hero_hint" >"$CSV"

# measure_open <phase> <index> <song> <album> <artist>: assumes the UI is at
# home. Sends open_song_album, waits for the page hero to become paintable,
# records a CSV row, then goes back and settles. Never aborts the run for a
# single timed-out album.
measure_open() {
    local phase=$1 index=$2 song=$3 album=$4 artist=$5
    local offset send_ms send_done_ms hero_epoch latency hint covers=0 hero=0
    offset=$(log_size)
    send_ms=$(now_ms)
    send_cmd "open_song_album $song"
    send_done_ms=$(now_ms)

    hero_epoch=""
    deadline=$((send_ms + HERO_TIMEOUT * 1000))
    while (( $(now_ms) < deadline )); do
        kill -0 "$UI_PID" 2>/dev/null || { tail -30 "$UI_LOG" >&2; die "UI exited while opening album $index"; }
        hero_epoch=$(first_hero_epoch "$offset" "$send_ms")
        [[ -n "$hero_epoch" ]] && break
        sleep "$POLL"
    done

    if [[ -n "$hero_epoch" ]]; then
        # A short grace lets the rest of the ready batch reach the log without
        # collecting many later repaint frames.
        sleep "$READY_GRACE"
        read -r covers hero <<<"$(count_ready "$offset")"
        latency=$(( hero_epoch - send_ms ))
        if (( latency <= HIT_MS )); then
            hint="fast_or_cached"
        else
            hint="decode"
        fi
    else
        read -r covers hero <<<"$(count_ready "$offset")"
        latency=""
        hint="timeout"
        FAILURES=$((FAILURES + 1))
        echo "check-cover-load:   $phase $index TIMEOUT after ${HERO_TIMEOUT}s (no page-hero ready line)" >&2
    fi

    printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
        "$phase" "$index" "$song" "$(sanitize "$album")" "$(sanitize "$artist")" \
        "$send_ms" "$send_done_ms" "$((send_done_ms - send_ms))" \
        "${hero_epoch:-}" "${latency:-}" "$hero" "$covers" "$hint" >>"$CSV"

    sleep "$SETTLE"
    send_cmd "back"
    sleep "$BACK_SETTLE"
}

echo "check-cover-load: opening $TOTAL album pages"
i=0
for row in "${SONG_ROWS[@]}"; do
    i=$((i + 1))
    IFS=$'\t' read -r song album_id album artist <<<"$row"
    measure_open open "$i" "$song" "$album" "$artist"
    if (( i % 5 == 0 )); then
        last=$(awk -F, -v i="$i" '$1=="open" && $2==i {print $10}' "$CSV")
        echo "check-cover-load:   $i/$TOTAL open (last hero ${last:-timeout} ms, failures $FAILURES)"
    fi
done

if (( REVISITS > 0 )); then
    IFS=$'\t' read -r song album_id album artist <<<"${SONG_ROWS[0]}"
    echo "check-cover-load: revisit phase ($REVISITS reopen(s) of album 1: $song)"
    for ((k = 1; k <= REVISITS; k++)); do
        measure_open revisit "$k" "$song" "$album" "$artist"
    done
fi

if (( BOUNCE > 0 )); then
    IFS=$'\t' read -r song album_id album artist <<<"${SONG_ROWS[0]}"
    echo "check-cover-load: bounce phase (open -> back -> reopen the same album)"
    measure_open bounce1 1 "$song" "$album" "$artist"
    measure_open bounce2 2 "$song" "$album" "$artist"
fi

revisit_song=""
if (( REVISITS > 0 )); then
    revisit_song=$song
fi

echo "check-cover-load: cleanup"
trap - EXIT INT TERM
cleanup

# ---- compact report -------------------------------------------------------
# Nearest-rank percentile over the sorted non-empty latency column ($10).
latency_stats() { # phase -> "n=.. min=.. p50=.. p95=.. max=.. mean=.."
    local values
    values=$(awk -F, -v p="$1" '$1 == p && $10 != "" { print $10 }' "$CSV" | sort -n)
    if [[ -z "$values" ]]; then
        echo "n=0"
        return 0
    fi
    awk '{ v[NR] = $1; sum += $1 }
        END {
            n = NR
            printf "n=%d min=%d p50=%d p95=%d max=%d mean=%.1f",
                n, v[1], v[int((n - 1) * 0.5 + 0.5) + 1],
                v[int((n - 1) * 0.95 + 0.5) + 1], v[n], sum / n
        }' <<<"$values"
}

mean_latency() { # phase
    awk -F, -v p="$1" '$1 == p && $10 != "" { sum += $10; n++ }
        END { if (n == 0) { print "n/a"; exit } printf "%.1f", sum / n }' "$CSV"
}

open_total=$(awk -F, '$1 == "open" { n++ } END { print n + 0 }' "$CSV")
open_ok=$(awk -F, '$1 == "open" && $10 != "" { n++ } END { print n + 0 }' "$CSV")
open_timeouts=$((open_total - open_ok))
open_fast=$(awk -F, '$1 == "open" && $13 == "fast_or_cached" { n++ } END { print n + 0 }' "$CSV")
revisit_total=$(awk -F, '$1 == "revisit" { n++ } END { print n + 0 }' "$CSV")
revisit_ok=$(awk -F, '$1 == "revisit" && $10 != "" { n++ } END { print n + 0 }' "$CSV")
hero_lines_stats=$(awk -F, '$1 == "open" { n++; sum += $11; if ($11 > max) max = $11 }
    END { if (n == 0) { print "n/a"; exit } printf "mean=%.1f max=%d", sum / n, max }' "$CSV")
cover_lines_stats=$(awk -F, '$1 == "open" { n++; sum += $12; if ($12 > max) max = $12 }
    END { if (n == 0) { print "n/a"; exit } printf "mean=%.1f max=%d", sum / n, max }' "$CSV")
send_overhead_stats=$(awk -F, '$1 == "open" { n++; sum += $8; if ($8 > max) max = $8 }
    END { if (n == 0) { print "n/a"; exit } printf "mean=%.2f max=%d", sum / n, max }' "$CSV")

paired_first=$(awk -F, -v s="$revisit_song" '$1 == "open" && $3 == s && $10 != "" { print $10; exit }' "$CSV")
paired_revisit=$(awk -F, -v s="$revisit_song" '$1 == "revisit" && $3 == s && $10 != "" { print $10; exit }' "$CSV")

open_stats=$(latency_stats open)
revisit_stats=$(latency_stats revisit)

{
    echo
    echo "Cover-load latency summary"
    echo "  sandbox   : $OUTDIR"
    echo "  binaries  : $BIN_DIR"
    echo "  csv       : $CSV"
    echo "startup (wall clock, ms)"
    echo "  daemon ready    : ${daemon_ready_ms}  (spawn -> both IPC sockets)"
    echo "  UI first frame  : ${ui_first_frame_ms}  (spawn -> first [musika/frame] summary)"
    echo "  first home cover: ${first_home_cover_ms}  (${first_cover_id} index=${first_cover_index} edge=${first_cover_edge})"
    echo "album hero (open_song_album send -> first [musika/cover] ready id=page-hero)"
    echo "  first opens     : ${open_stats}"
    echo "  samples         : ${open_ok}/${open_total} ok, ${open_timeouts} timeout, ${open_fast} fast/cached (<= ${HIT_MS} ms)"
    echo "  ready lines/open: hero ${hero_lines_stats}, all covers ${cover_lines_stats}"
    echo "  send overhead   : ${send_overhead_stats} ms (harness -> ipc.sock)"
    echo "revisit"
    echo "  reopen samples  : ${revisit_stats}"
    echo "  samples         : ${revisit_ok}/${revisit_total} ok"
    echo "  means           : first-open $(mean_latency open) ms, reopen $(mean_latency revisit) ms"
    if [[ -n "$revisit_song" ]]; then
        echo "  paired          : song ${revisit_song}: first=${paired_first:-timeout} ms, reopen=${paired_revisit:-timeout} ms"
    fi
    if (( BOUNCE > 0 )); then
        echo "bounce (same album reopened immediately; warm-cover retention)"
        echo "  first open      : $(latency_stats bounce1)"
        echo "  immediate reopen: $(latency_stats bounce2)"
    fi
    echo "  frame log       : $(frame_lines) lines, UI log $(du -h "$UI_LOG" 2>/dev/null | cut -f1)"
    last_active=$(grep '^\[musika/frame\]' "$UI_LOG" 2>/dev/null | tail -1 | sed -n 's/.*active=\([01]\).*/\1/p')
    echo "  window active   : ${last_active:-unknown} (must be 1; hidden windows are frame-throttled)"
    echo "  logs            : $DAEMON_LOG $UI_LOG"
} | tee "$SUMMARY"

if (( FAILURES > 0 )); then
    echo "check-cover-load: completed with $FAILURES timed-out album(s); see $CSV" >&2
else
    echo "check-cover-load: completed without failures" >&2
fi

exit 0
