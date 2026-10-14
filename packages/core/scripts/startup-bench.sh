#!/bin/bash
# How fast Vorn opens on macOS, from process start to the window on screen, to
# vornd connected, to content (the window up and its grid connected), in five states:
#
#   empty           a fresh data directory, no vornd: the app starts one
#   empty-warm      a fresh data directory, vornd already running
#   seeded-warm     10 projects, 500 tasks, 22 workflows and 30 live sessions
#                   of 8,000 lines each, vornd and its session holder running
#   seeded-cold     the same with vornd stopped and the holder still running
#   quarantined     a fresh quarantined copy of VORN_APP_BUNDLE (a built
#                   Vorn.app), as a first launch after download; skipped without it
#
# Each state runs RUNS times (5); the table shows medians in milliseconds. The
# warm states are held to BUDGET_WINDOW_MS (200) and BUDGET_CONTENT_MS (500):
# over budget exits 1 unless STARTUP_BENCH_REPORT_ONLY=1. Everything runs under a
# temporary HOME and data directory; nothing touches ~/.vorn.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname)" = Darwin ] || { echo "startup-bench: macOS only" >&2; exit 0; }

RUNS=${RUNS:-5}
BUDGET_WINDOW_MS=${BUDGET_WINDOW_MS:-200}
BUDGET_CONTENT_MS=${BUDGET_CONTENT_MS:-500}
T=$PWD/target/ci
WORK=$(mktemp -d /tmp/vorn-startup.XXXXXX)
trap 'pkill -f "$WORK/" 2>/dev/null || true; sleep 1; rm -rf "$WORK" 2>/dev/null || true' EXIT

if [ "${SKIP_BUILD:-}" != 1 ]; then
  cargo build --profile ci --locked -p vornd -p vorn-sessiond -p vorn-app --bin vorn-app >&2
fi
swiftc -O scripts/startup/winwait.swift -o "$WORK/winwait" >&2

now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }

# The environment Finder gives an app: little more than HOME. It replaces the
# calling process, so a `( run_env ... ) &` has the program's own pid.
run_env() {
  exec env -i HOME="$1/home" USER="$USER" PATH=/usr/bin:/bin:/usr/sbin:/sbin SHELL=/bin/zsh \
    TMPDIR="$1/" VORN_TAILSCALE=none VORND_KEYCHAIN=0 "${@:2}"
}

# A fresh root: its HOME, data directory, and an agent on the shell's PATH.
fresh() {
  rm -rf "$1"
  mkdir -p "$1/home" "$1/d" "$1/bin"
  echo "export PATH=$1/bin:\$PATH" > "$1/home/.zshenv"
}

# Starts vornd on root $1 as the app would, and waits for it to listen.
start_vornd() {
  (run_env "$1" "$T/vornd" --data-dir "$1/d" --sessiond "$T/vorn-sessiond" \
    --log-file "$1/d/server.log" > /dev/null 2>&1) &
  for _ in $(seq 1 200); do [ -s "$1/d/ws-port" ] && [ -s "$1/d/local-token" ] && return; sleep 0.05; done
  echo "startup-bench: vornd did not start" >&2
  exit 1
}

# One launch of the app (program $2...) on root $1: "window connected content" in ms.
once() {
  local root=$1; shift
  local log=$root/d/server.log
  local before=0
  [ -f "$log" ] && before=$(wc -l < "$log")
  local t0; t0=$(now)
  (run_env "$root" "$@" window --data-dir "$root/d" > "$root/app.out" 2>&1) &
  local pid=$!
  local tw; tw=$("$WORK/winwait" "$pid" 30 || true)
  # The app's RPC socket and its grid socket, as vornd logs them.
  for _ in $(seq 1 300); do
    [ -f "$log" ] && tail -n +"$((before + 1))" "$log" | grep -q "grid client" && break
    sleep 0.05
  done
  sleep 0.2
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  tail -n +"$((before + 1))" "$log" 2>/dev/null | python3 -I -c '
import sys, re, datetime
t0, tw = float(sys.argv[1]), sys.argv[2]
win = (float(tw) - t0) * 1000 if tw not in ("", "none") else float("nan")
at = {}
for line in sys.stdin:
    line = re.sub(r"\x1b\[[0-9;]*m", "", line)
    m = re.match(r"(\S+Z)\s+\w+\s+(.*)", line)
    if not m:
        continue
    t = datetime.datetime.fromisoformat(m.group(1).replace("Z", "+00:00")).timestamp()
    for key, needle in (("connected", "a client connected"), ("content", "grid client")):
        if needle in m.group(2) and key not in at:
            at[key] = (t - t0) * 1000
# Content shows once both the window and the grid are there.
content = max(win, at.get("content", float("nan")))
print("%.0f %.0f %.0f" % (win, at.get("connected", float("nan")), content))
' "$t0" "$tw"
}

# RUNS launches of state $1: one line of medians.
measure() {
  local name=$1 root=$2; shift 2
  local rows=()
  for _ in $(seq 1 "$RUNS"); do
    [ "$name" = empty ] && { pkill -f "$root/" 2>/dev/null || true; sleep 0.5; fresh "$root"; }
    [ "$name" = seeded-cold ] && { pkill -f "vornd --data-dir $root/d" 2>/dev/null || true; sleep 0.5; }
    rows+=("$(once "$root" "$@")")
  done
  printf '%s\n' "${rows[@]}" | python3 -I -c '
import sys, statistics
name = sys.argv[1]
rows = [[float(x) for x in l.split()] for l in sys.stdin if l.strip()]
def med(i):
    vals = [r[i] for r in rows if r[i] == r[i]]
    return statistics.median(vals) if vals else float("nan")
print("%-12s %8.0f %10.0f %8.0f" % (name, med(0), med(1), med(2)))
' "$name"
}

echo "state          window  connected  content   (ms, median of $RUNS)"
results=()
E=$WORK/empty; fresh "$E"
results+=("$(measure empty "$E" "$T/vorn-app")")
EW=$WORK/emptywarm; fresh "$EW"; start_vornd "$EW"
results+=("$(measure empty-warm "$EW" "$T/vorn-app")")
S=$WORK/seeded; fresh "$S"; start_vornd "$S"
node scripts/startup/seed.mjs "$S/d" "$S" >&2
sleep 3
results+=("$(measure seeded-warm "$S" "$T/vorn-app")")
results+=("$(measure seeded-cold "$S" "$T/vorn-app")")
if [ -n "${VORN_APP_BUNDLE:-}" ]; then
  Q=$WORK/quarantined; fresh "$Q"
  cp -R "$VORN_APP_BUNDLE" "$Q/Vorn.app"
  xattr -w com.apple.quarantine "0081;$(printf %x "$(date +%s)");Safari;" "$Q/Vorn.app"
  results+=("$(RUNS=1 measure quarantined "$Q" "$Q/Vorn.app/Contents/MacOS/Vorn")")
fi
printf '%s\n' "${results[@]}"

# The warm states are held to the budgets; first launches are only reported.
over=$(printf '%s\n' "${results[@]}" | awk -v w="$BUDGET_WINDOW_MS" -v c="$BUDGET_CONTENT_MS" '
  ($1 == "empty-warm" || $1 == "seeded-warm") && ($2 > w || $4 > c) { print $1 }')
if [ -n "$over" ]; then
  echo "over budget (window ${BUDGET_WINDOW_MS} ms, content ${BUDGET_CONTENT_MS} ms): $over" >&2
  [ "${STARTUP_BENCH_REPORT_ONLY:-}" = 1 ] || exit 1
fi
