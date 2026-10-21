#!/usr/bin/env bash
# The extension load test on a bench host: builds the test daemons, the CLI
# (vornd compiles linked Wasm components with it) and the many-pane bench,
# then runs 32 busy panes with no extensions and with 20, at the default
# idle time and cap, at other caps, and through an idle phase.
#
#   bench/ext-load/run.sh OUT_DIR
#
# Env: REPS (5), RUN_SECONDS (20), CAPS ("12 16 24"), IDLE_MS (60000),
#      IDLE_CAP (unset: the default), IDLE_WAIT (120), JOBS (cargo's -j).
set -euo pipefail

out=${1:?usage: run.sh OUT_DIR}
mkdir -p "$out"
cd "$(dirname "$0")/../.."
export PATH="$HOME/zig:$HOME/.cargo/bin:$PATH"
jobs=${JOBS:+-j $JOBS}

# shellcheck disable=SC2086
cargo build --profile ci --locked $jobs -p vornd -p vorn-sessiond -p vorn-cli --bins >"$out/build.log" 2>&1
# shellcheck disable=SC2086
cargo build --profile ci --locked $jobs -p vorn-app --example many_panes >>"$out/build.log" 2>&1

bench=target/ci/examples/many_panes
common=(--panes 32 --cases agent --seconds "${RUN_SECONDS:-20}" --ext-period 2)
reps=${REPS:-5}

echo "default idle time and cap"
"$bench" "${common[@]}" --extensions 0,20 --reps "$reps" --out "$out/default.json" >"$out/default.md"
for cap in ${CAPS:-12 16 24}; do
  echo "cap $cap"
  "$bench" "${common[@]}" --extensions 20 --reps 3 --ext-cap "$cap" --out "$out/cap-$cap.json" >"$out/cap-$cap.md"
done
echo "idle phase"
idle_cap=${IDLE_CAP:+--ext-cap $IDLE_CAP}
# shellcheck disable=SC2086
"$bench" "${common[@]}" --extensions 0,20 --reps 3 $idle_cap --ext-idle-ms "${IDLE_MS:-60000}" \
  --idle-wait "${IDLE_WAIT:-120}" --out "$out/idle.json" >"$out/idle.md"
{
  uname -a
  nproc
  grep -m1 'model name' /proc/cpuinfo || true
  free -m
} >"$out/machine.txt"
echo "done"
