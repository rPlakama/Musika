#!/usr/bin/env bash
# Sandbox-only construction budget harness for the Musika core daemon.
#
# Measures the daemon's *construction* cost, not UI/playback work:
#   * socket-ready latency (spawn -> daemon.sock present), p50/p95
#   * fresh resident set after settling: VmRSS, RssAnon, RssFile, mappings
#   * thread census (names + count), sqlx worker count
#   * idle CPU ticks and voluntary context switches (wakeups) over a sample
#   * per-thread idle breakdown (who wakes/costs what)
#   * PipeWire node state while idle (running pool vs suspended)
#   * static address-space facts: binary size and `ldd` library count
#   * `musika --version` p50 (address-space construction proxy: loader + pages)
#
# SAFETY: this script NEVER uses the user's real config/cache. It copies
# $MUSIKA_LEAN_CONFIG_SRC (default /tmp/musika-mem/config-src, itself a copy of
# ~/.config/musika) into <outdir>/sample-N/config/musika, points XDG_CONFIG_HOME
# and XDG_CACHE_HOME at per-sample dirs, and uses private sockets. It only ever
# kills the daemon/private D-Bus it spawned itself. It refuses to run against
# the real config.
#
# Usage:
#   scripts/check-daemon-lean.sh [bin-dir] [samples] [outdir]
#
#   bin-dir   directory holding `musika` (default target/release)
#   samples   number of daemon runs (default 3)
#   outdir    output dir (default /tmp/musika-lean/run-<stamp>)
#
# Env overrides:
#   MUSIKA_LEAN_CONFIG_SRC   pristine config snapshot (never the real config)
#   MUSIKA_LEAN_SETTLE       seconds after socket-ready before sampling (3)
#   MUSIKA_LEAN_IDLE         seconds of idle sampling per run (20)
#   MUSIKA_LEAN_REAP_WAIT    extra seconds before the second sqlx sample (0)
#   MUSIKA_LEAN_DBUS         1 to run a private session bus for tray/MPRIS (1)
#   MUSIKA_LEAN_COVERS       1 to hardlink the snapshot's cover cache (1; makes
#                            the restored queue resolve covers without network)
#   MUSIKA_LEAN_PW           1 to record the PipeWire node state (1)
#   MUSIKA_LEAN_VERSION_RUNS `musika --version` samples (10)
#   Regression gates (exit 1 when any exceeded):
#   MUSIKA_LEAN_MAX_READY_MS, MUSIKA_LEAN_MAX_RSS_KIB,
#   MUSIKA_LEAN_MAX_THREADS, MUSIKA_LEAN_MAX_VCSW_PER_SEC
set -euo pipefail
export LC_ALL=C

die() { echo "check-daemon-lean: $*" >&2; exit 1; }
now_ms() { date +%s%3N; }

ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
BIN_DIR=$(cd "${1:-$ROOT/target/release}" 2>/dev/null && pwd) || die "bin dir not found: ${1:-$ROOT/target/release}"
SAMPLES=${2:-3}
[[ "$SAMPLES" =~ ^[0-9]+$ ]] || die "samples must be a number: $SAMPLES"
(( SAMPLES > 0 )) || die "samples must be > 0"
STAMP=$(date +%Y%m%d-%H%M%S)
OUTDIR=${3:-/tmp/musika-lean/run-$STAMP}
mkdir -p "$OUTDIR"
OUTDIR=$(cd "$OUTDIR" && pwd)

[[ -x "$BIN_DIR/musika" ]] || die "missing executable: $BIN_DIR/musika"

SRC=${MUSIKA_LEAN_CONFIG_SRC:-/tmp/musika-mem/config-src}
[[ -f "$SRC/library.db" ]] || die "config snapshot not found: $SRC/library.db (set MUSIKA_LEAN_CONFIG_SRC)"
SRC_ABS=$(cd "$SRC" && pwd -P)
REAL_CFG="$HOME/.config/musika"
case "$SRC_ABS/" in
    "$REAL_CFG/"*) die "refusing to run against the user's real config ($SRC_ABS)" ;;
esac
case "$OUTDIR/" in
    "$HOME/.config/"*) die "outdir must not live under ~/.config ($OUTDIR)" ;;
esac

SETTLE=${MUSIKA_LEAN_SETTLE:-3}
IDLE=${MUSIKA_LEAN_IDLE:-20}
REAP_WAIT=${MUSIKA_LEAN_REAP_WAIT:-0}
USE_DBUS=${MUSIKA_LEAN_DBUS:-1}
USE_PW=${MUSIKA_LEAN_PW:-1}
VERSION_RUNS=${MUSIKA_LEAN_VERSION_RUNS:-10}
MAX_READY_MS=${MUSIKA_LEAN_MAX_READY_MS:-0}
MAX_RSS_KIB=${MUSIKA_LEAN_MAX_RSS_KIB:-0}
MAX_THREADS=${MUSIKA_LEAN_MAX_THREADS:-0}
MAX_VCSW_PER_SEC=${MUSIKA_LEAN_MAX_VCSW_PER_SEC:-0}
[[ "$SETTLE" =~ ^[0-9]+$ ]] || die "MUSIKA_LEAN_SETTLE must be whole seconds"
[[ "$IDLE" =~ ^[0-9]+$ ]] || die "MUSIKA_LEAN_IDLE must be whole seconds"
[[ "$REAP_WAIT" =~ ^[0-9]+$ ]] || die "MUSIKA_LEAN_REAP_WAIT must be whole seconds"
[[ "$VERSION_RUNS" =~ ^[0-9]+$ ]] || die "MUSIKA_LEAN_VERSION_RUNS must be a number"

CSV="$OUTDIR/daemon-lean.csv"
SUMMARY="$OUTDIR/summary.txt"
BUS_PID=""
DAEMON_PID=""

cleanup() {
    set +e
    [[ -n "$DAEMON_PID" ]] && kill "$DAEMON_PID" 2>/dev/null
    [[ -n "$BUS_PID" ]] && kill "$BUS_PID" 2>/dev/null
    [[ -n "$DAEMON_PID" ]] && wait "$DAEMON_PID" 2>/dev/null
    [[ -n "$BUS_PID" ]] && wait "$BUS_PID" 2>/dev/null
    set -e
}
trap cleanup EXIT INT TERM

# --- static address-space facts -------------------------------------------
# Nix's wrapper scripts are not ELF: measure the wrapped binary's closure.
BIN_ELF="$BIN_DIR/musika"
if [[ -x "$BIN_DIR/.musika-wrapped" ]]; then
    BIN_ELF="$BIN_DIR/.musika-wrapped"
fi
BIN_SIZE=$(stat -c%s "$BIN_ELF" 2>/dev/null || echo 0)
LDD_COUNT=0
if command -v ldd >/dev/null 2>&1; then
    LDD_COUNT=$(ldd "$BIN_ELF" 2>/dev/null | grep -c '=>' || true)
fi
version_p50_ms=$(python3 - "$BIN_ELF" "$VERSION_RUNS" <<'PY'
import subprocess, sys, time
binary, runs = sys.argv[1], int(sys.argv[2])
samples = []
for _ in range(runs):
    start = time.perf_counter_ns()
    subprocess.run([binary, "--version"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    samples.append((time.perf_counter_ns() - start) / 1e6)
samples.sort()
print(f"{samples[len(samples) // 2]:.2f}")
PY
)

# --- optional private session bus (tray/MPRIS) ----------------------------
if [[ "$USE_DBUS" == "1" ]] && command -v dbus-daemon >/dev/null 2>&1; then
    BUS_ADDR="unix:path=$OUTDIR/bus.sock"
    dbus-daemon --session --nofork --address="$BUS_ADDR" --nopidfile >"$OUTDIR/bus.log" 2>&1 &
    BUS_PID=$!
    for _ in $(seq 1 100); do
        [[ -S "$OUTDIR/bus.sock" ]] && break
        sleep 0.01
    done
    [[ -S "$OUTDIR/bus.sock" ]] || die "private dbus-daemon did not start"
else
    BUS_ADDR=""
fi

echo "check-daemon-lean: bin=$BIN_DIR samples=$SAMPLES idle=${IDLE}s out=$OUTDIR"
echo "static: binary=${BIN_SIZE}B ldd_libs=$LDD_COUNT version_p50=${version_p50_ms}ms"

echo "sample,ready_ms,rss_kib,anon_kib,file_kib,vm_size_kib,mappings,threads,sqlx_threads,reaped_sqlx_threads,pw_state,cpu_ticks,idle_s,vcsw,vcsw_per_s" >"$CSV"

for sample in $(seq 1 "$SAMPLES"); do
    SAMPLE_DIR="$OUTDIR/sample-$sample"
    CFG="$SAMPLE_DIR/config"
    CACHE="$SAMPLE_DIR/cache"
    HOME_DIR="$SAMPLE_DIR/home"
    rm -rf "$SAMPLE_DIR"
    mkdir -p "$CFG" "$CACHE" "$HOME_DIR"
    # `XDG_CONFIG_HOME` is the app config dir itself (not a parent): the binary
    # resolves `library.db`, `accounts.json` and `key` directly under it, the
    # same layout the launcher uses for `~/.config/musika`.
    cp "$SRC/library.db" "$SRC/media.db" "$SRC/accounts.json" "$SRC/key" "$CFG/" 2>/dev/null || true
    [[ -f "$SRC/scroll.json" ]] && cp "$SRC/scroll.json" "$CFG/" || true
    # The restored queue resolves its cover URLs from the copied cover cache;
    # omitting it would make the daemon fetch every cover on startup, which is
    # not construction cost. Hardlinks keep the sample copy cheap.
    if [[ "${MUSIKA_LEAN_COVERS:-1}" != "0" && -d "$SRC/covers" ]]; then
        cp -al "$SRC/covers" "$CFG/covers" 2>/dev/null \
            || cp -a "$SRC/covers" "$CFG/covers" 2>/dev/null || true
    fi
    IPC="$SAMPLE_DIR/ipc.sock"
    DAEMON_SOCK="$SAMPLE_DIR/daemon.sock"

    spawn_ms=$(now_ms)
    env -u MUSIKA_IPC_SOCKET -u MUSIKA_DAEMON_SOCKET \
        XDG_CONFIG_HOME="$CFG" XDG_CACHE_HOME="$CACHE" HOME="$HOME_DIR" \
        MUSIKA_ASSET_DIR="$ROOT/src" \
        MUSIKA_IPC_SOCKET="$IPC" MUSIKA_DAEMON_SOCKET="$DAEMON_SOCK" \
        DBUS_SESSION_BUS_ADDRESS="$BUS_ADDR" \
        "$BIN_ELF" --daemon >"$SAMPLE_DIR/daemon.log" 2>&1 &
    DAEMON_PID=$!

    ready_ms=""
    for _ in $(seq 1 30000); do
        if [[ -S "$DAEMON_SOCK" ]]; then
            ready_ms=$(( $(now_ms) - spawn_ms ))
            break
        fi
        kill -0 "$DAEMON_PID" 2>/dev/null || { tail -20 "$SAMPLE_DIR/daemon.log" >&2; die "daemon exited before socket ready"; }
        sleep 0.001
    done
    [[ -n "$ready_ms" ]] || { tail -20 "$SAMPLE_DIR/daemon.log" >&2; die "daemon socket not ready within 30s"; }

    sleep "$SETTLE"
    kill -0 "$DAEMON_PID" 2>/dev/null || die "daemon exited before sampling"

    rss_kib=$(awk '/^VmRSS:/{print $2}' "/proc/$DAEMON_PID/status")
    anon_kib=$(awk '/^RssAnon:/{print $2}' "/proc/$DAEMON_PID/status")
    file_kib=$(awk '/^RssFile:/{print $2}' "/proc/$DAEMON_PID/status")
    vm_size_kib=$(awk '/^VmSize:/{print $2}' "/proc/$DAEMON_PID/status")
    threads=$(awk '/^Threads:/{print $2}' "/proc/$DAEMON_PID/status")
    mappings=$(wc -l <"/proc/$DAEMON_PID/maps")
    sqlx_threads=$(for t in /proc/"$DAEMON_PID"/task/*; do cat "$t/comm" 2>/dev/null; done | grep -c '^sqlx-sqlite-wor' || true)

    pw_state=$(python3 - "$DAEMON_PID" "$USE_PW" <<'PY'
import json, subprocess, sys
pid, enabled = sys.argv[1], sys.argv[2] == "1"
if not enabled:
    print("")
    raise SystemExit(0)
try:
    dump = json.loads(subprocess.run(["pw-dump"], capture_output=True, text=True, timeout=5).stdout)
except Exception:
    print("unavailable")
    raise SystemExit(0)
clients = {}
for obj in dump:
    if obj.get("type") != "PipeWire:Interface:Client":
        continue
    props = obj.get("info", {}).get("props", {})
    clients[obj.get("id")] = str(props.get("application.process.id", ""))
states = []
for obj in dump:
    if obj.get("type") != "PipeWire:Interface:Node":
        continue
    info = obj.get("info", {})
    props = info.get("props", {})
    if "Musika" not in str(props.get("application.name", "")):
        continue
    client = props.get("client.id")
    if client is None or clients.get(client) != pid:
        continue
    states.append(str(info.get("state", "?")))
print("|".join(sorted(set(states))))
PY
)

    # Per-thread idle census: one snapshot, IDLE seconds, second snapshot.
    idle_out=$(python3 - "$DAEMON_PID" "$IDLE" "$SAMPLE_DIR/threads.txt" <<'PY'
import os, sys, time
pid, seconds, thread_log = sys.argv[1], int(sys.argv[2]), sys.argv[3]
def snap():
    rows = {}
    try:
        tids = os.listdir(f"/proc/{pid}/task")
    except OSError:
        return rows
    for tid in tids:
        try:
            st = open(f"/proc/{pid}/task/{tid}/stat").read()
            comm = st.split("(", 1)[1].rsplit(")", 1)[0]
            fields = st.rsplit(") ", 1)[1].split()
            cpu = int(fields[11]) + int(fields[12])
            vcsw = 0
            for line in open(f"/proc/{pid}/task/{tid}/status"):
                if line.startswith("voluntary_ctxt_switches"):
                    vcsw = int(line.split()[1])
            rows[tid] = (comm, cpu, vcsw)
        except (OSError, IndexError, ValueError):
            # The thread exited between listdir and the per-file reads.
            continue
    return rows
before = snap()
time.sleep(seconds)
after = snap()
ticks = sum(after[t][1] for t in after if t in before) - sum(before[t][1] for t in before)
vcsw = sum(after[t][2] for t in after if t in before) - sum(before[t][2] for t in before)
per_thread = []
for tid, (comm, cpu, sw) in after.items():
    if tid not in before:
        continue
    dc, ds = cpu - before[tid][1], sw - before[tid][2]
    if dc or ds:
        per_thread.append((dc, ds, comm))
per_thread.sort(reverse=True)
with open(thread_log, "w") as log:
    for dc, ds, comm in per_thread:
        log.write(f"{dc:6d} ticks {ds:7d} vcsw  {comm}\n")
print(max(0, ticks), max(0, vcsw))
PY
)
    cpu_ticks=${idle_out%% *}
    vcsw=${idle_out##* }
    [[ "$cpu_ticks" =~ ^[0-9]+$ && "$vcsw" =~ ^[0-9]+$ ]] || die "idle sampling failed for sample $sample"
    vcsw_per_s=$(python3 -c "print(round($vcsw / $IDLE, 2))")

    reaped_sqlx_threads=""
    if (( REAP_WAIT > 0 )); then
        sleep "$REAP_WAIT"
        if kill -0 "$DAEMON_PID" 2>/dev/null; then
            reaped_sqlx_threads=$(for t in /proc/"$DAEMON_PID"/task/*; do cat "$t/comm" 2>/dev/null; done | grep -c '^sqlx-sqlite-wor' || true)
        fi
    fi

    echo "$sample,$ready_ms,$rss_kib,$anon_kib,$file_kib,$vm_size_kib,$mappings,$threads,$sqlx_threads,$reaped_sqlx_threads,$pw_state,$cpu_ticks,$IDLE,$vcsw,$vcsw_per_s" >>"$CSV"

    python3 - "$IPC" <<'PY'
import socket, sys, time
for _ in range(200):
    try:
        sock = socket.socket(socket.AF_UNIX)
        sock.settimeout(1)
        sock.connect(sys.argv[1])
        sock.sendall(b"quit\n")
        time.sleep(0.1)
        sock.close()
        break
    except OSError:
        time.sleep(0.02)
PY
    for _ in $(seq 1 100); do
        kill -0 "$DAEMON_PID" 2>/dev/null || break
        sleep 0.02
    done
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
    DAEMON_PID=""
    echo "check-daemon-lean: sample $sample ready=${ready_ms}ms rss=${rss_kib}KiB threads=${threads} sqlx=${sqlx_threads} pw=${pw_state:-n/a} vcsw/s=${vcsw_per_s}"
done

python3 - "$CSV" "$SUMMARY" "$BIN_SIZE" "$LDD_COUNT" "$version_p50_ms" <<'PY'
import csv, statistics, sys
csv_path, summary_path, bin_size, ldd_count, version_p50 = sys.argv[1:6]
rows = list(csv.DictReader(open(csv_path)))
def col(name):
    return [float(row[name]) for row in rows if row[name] not in ("", None)]
def med(values):
    return statistics.median(values) if values else 0.0
ready = col("ready_ms")
rss = col("rss_kib")
anon = col("anon_kib")
file_ = col("file_kib")
threads = col("threads")
vcsw = col("vcsw_per_s")
sqlx = col("sqlx_threads")
reaped = col("reaped_sqlx_threads")
pw = sorted({row["pw_state"] for row in rows if row["pw_state"]})
def p95(values):
    if not values:
        return 0.0
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(len(ordered) * 0.95))]
lines = [
    f"binary_bytes={bin_size}",
    f"ldd_libs={ldd_count}",
    f"version_p50_ms={version_p50}",
    f"ready_p50_ms={med(ready):.1f}",
    f"ready_p95_ms={p95(ready):.1f}",
    f"rss_median_kib={med(rss):.0f}",
    f"anon_median_kib={med(anon):.0f}",
    f"file_median_kib={med(file_):.0f}",
    f"threads_median={med(threads):.0f}",
    f"sqlx_workers_median={med(sqlx):.0f}",
]
if reaped:
    lines.append(f"sqlx_workers_reaped_median={med(reaped):.0f}")
lines += [
    f"idle_vcsw_per_s_median={med(vcsw):.2f}",
    f"pw_states={'|'.join(pw) if pw else 'n/a'}",
]
text = "\n".join(lines) + "\n"
open(summary_path, "w").write(text)
print("--- summary ---")
print(text, end="")
PY

# Optional regression gates.
python3 - "$SUMMARY" "$MAX_READY_MS" "$MAX_RSS_KIB" "$MAX_THREADS" "$MAX_VCSW_PER_SEC" <<'PY'
import sys
summary_path, max_ready, max_rss, max_threads, max_vcsw = sys.argv[1:]
values = dict(line.strip().split("=", 1) for line in open(summary_path) if "=" in line)
def num(key):
    try:
        return float(values.get(key, 0))
    except ValueError:
        return 0.0
checks = [
    ("ready_p50_ms", float(max_ready)),
    ("rss_median_kib", float(max_rss)),
    ("threads_median", float(max_threads)),
    ("idle_vcsw_per_s_median", float(max_vcsw)),
]
failed = [f"{key}={num(key)} > {limit}" for key, limit in checks if limit and num(key) > limit]
if failed:
    print("check-daemon-lean: gate failed: " + ", ".join(failed), file=sys.stderr)
    raise SystemExit(1)
PY

echo "check-daemon-lean: results in $OUTDIR"
