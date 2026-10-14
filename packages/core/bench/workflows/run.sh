#!/usr/bin/env bash
# Builds this checkout's vornd, vorn-sessiond and the bench, runs every case
# against it and, when BEFORE_BIN names a directory holding another build's
# vornd and vorn-sessiond, against that one too, case by case in turn; then
# writes OUT_DIR/summary.md. Every vornd runs on its own temporary HOME and
# --data-dir.
#
#   run.sh OUT_DIR             REPS=3 by default; ONLY=flows,gates,schedule
set -euo pipefail

out=${1:?usage: run.sh OUT_DIR}
core="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$out"

cd "$core"
nice -n 10 cargo build --profile ci --locked -j 3 -p vornd -p vorn-sessiond -p vorn-workflow-bench
bench="$core/target/ci/vorn-workflow-bench"

bins=()
if [ -n "${BEFORE_BIN:-}" ]; then
  bins+=(--bin "before=$BEFORE_BIN")
fi
bins+=(--bin "after=$core/target/ci")

"$bench" run --out "$out" "${bins[@]}" --reps "${REPS:-3}" --only "${ONLY:-flows,gates,schedule}"
"$bench" report "$out"
