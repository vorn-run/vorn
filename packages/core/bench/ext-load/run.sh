#!/usr/bin/env bash
# The extension load test on a bench host: builds the test daemons, the CLI
# (vornd compiles linked Wasm components with it), the Rust template's
# program and the many-pane bench, then runs 32 busy panes with no
# extensions and with 20 (generated from templates/), at the default idle
# time and cap, at other caps, and through an idle phase.
#
#   bench/ext-load/run.sh OUT_DIR
#
# Env: REPS (5), RUN_SECONDS (20), CAPS ("16 20 32"), IDLE_MS (unset: the
#      default), IDLE_CAP (unset: the default), IDLE_WAIT (150), JOBS (cargo's -j).
set -euo pipefail

out=${1:?usage: run.sh OUT_DIR}
mkdir -p "$out"
cd "$(dirname "$0")/../.."
export PATH="$HOME/zig:$HOME/.cargo/bin:$PATH"
# Vorn's own extensions are part of the load: a vornd without them fails to build.
export VORN_SHIPPED_REQUIRED=1
jobs=${JOBS:+-j $JOBS}

# shellcheck disable=SC2086
cargo build --profile ci --locked $jobs -p vornd -p vorn-sessiond -p vorn-cli --bins >"$out/build.log" 2>&1
# shellcheck disable=SC2086
cargo build --profile ci --locked $jobs -p vorn-bench-ext-tiers --bin tier-load >>"$out/build.log" 2>&1
# shellcheck disable=SC2086
cargo build --profile ci --locked $jobs -p vorn-app --example many_panes >>"$out/build.log" 2>&1

bench=target/ci/examples/many_panes
common=(--panes 32 --cases agent --seconds "${RUN_SECONDS:-20}" --ext-period 2)
reps=${REPS:-5}

# A step that fails is reported and the rest still run; the script then fails.
failed=0
step() {
  local name=$1
  shift
  echo "$name"
  "$bench" "${common[@]}" "$@" --out "$out/$name.json" >"$out/$name.md" 2>"$out/$name.err" || {
    echo "$name failed: $(tail -n 1 "$out/$name.err")"
    failed=1
  }
}

step default --extensions 0,20 --reps "$reps"
for cap in ${CAPS:-16 20 32}; do
  step "cap-$cap" --extensions 20 --reps 3 --ext-cap "$cap"
done
idle_cap=${IDLE_CAP:+--ext-cap $IDLE_CAP}
idle_ms=${IDLE_MS:+--ext-idle-ms $IDLE_MS}
# shellcheck disable=SC2086
step idle --extensions 0,20 --reps 3 $idle_cap $idle_ms --idle-wait "${IDLE_WAIT:-150}"
{
  uname -a
  nproc
  grep -m1 'model name' /proc/cpuinfo || true
  free -m
} >"$out/machine.txt"
echo "done"
exit "$failed"
