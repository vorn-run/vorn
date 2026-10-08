#!/usr/bin/env bash
# Builds the release binaries and runs each tier on a bench host, writing
# tier-N.json and tier-N.log into $OUT. A tier that fails still leaves its
# JSON with what it measured and the limit it hit.
#
#   run-tiers.sh OUT_DIR [TIER...]     (default tiers: 100 1000 10000)
#   PHASES=holder,stack picks the phases (default both).
set -euo pipefail

out=${1:?usage: run-tiers.sh OUT_DIR [TIER...]}
shift
tiers=("${@:-100 1000 10000}")
read -r -a tiers <<<"${tiers[*]}"

export PATH="$HOME/zig:$HOME/.cargo/bin:$PATH"
core="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$out"

cd "$core"
if ! cargo build --release --locked -p vornd -p vorn-sessiond -p vorn-scale-bench >"$out/build.log" 2>&1; then
  grep -A 20 '^error' "$out/build.log" | head -n 60
  exit 1
fi
bin="$core/target/release"

ulimit -n 1048576
ulimit -u unlimited
{
  echo "nofile: $(ulimit -n)  nproc: $(ulimit -u)"
  sysctl kernel.pty.max kernel.pid_max kernel.threads-max vm.max_map_count
  systemctl show "user-$(id -u).slice" -p TasksMax
  systemctl show "user@$(id -u).service" -p TasksMax
  nproc
  free -g
} >"$out/host.txt" 2>&1

for tier in "${tiers[@]}"; do
  work="/tmp/vorn-scale-$tier"
  rm -rf "$work"
  # Generous: 10,000 sessions spawn, settle and hand off twice over.
  limit=$((900 + tier / 4))
  echo "tier $tier (timeout ${limit}s)"
  VORN_BENCH_HOST=1 timeout --kill-after=30 "$limit" "$bin/vorn-scale-bench" run \
    --tier "$tier" \
    --phases "${PHASES:-holder,stack}" \
    --sessiond "$bin/vorn-sessiond" \
    --vornd "$bin/vornd" \
    --work "$work" \
    --out "$out/tier-$tier.json" >"$out/tier-$tier.log" 2>&1 ||
    echo "tier $tier exited with $?" | tee -a "$out/tier-$tier.log"
  tail -n 5 "$out/tier-$tier.log"
  # Whatever a killed run left behind, so the next tier starts clean.
  for p in $(pgrep -x vorn-sessiond; pgrep -x vornd; pgrep -f 'vorn-scale-bench buildlog'); do
    kill -9 "$p" 2>/dev/null || true
  done
  rm -rf "$work"
done
