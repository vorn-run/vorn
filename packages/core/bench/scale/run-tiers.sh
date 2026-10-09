#!/usr/bin/env bash
# Builds the release binaries and runs each tier on a bench host, writing
# tier-N.json and tier-N.log into $OUT. A tier that fails still leaves its
# JSON with what it measured and the limit it hit.
#
#   run-tiers.sh OUT_DIR [TIER...]     (default tiers: 100 1000 10000)
#   PHASES=holder,stack picks the phases (default both).
#   TESTS=1 first runs the holder's, vornd's and the engine's tests into tests.log.
#   COMPARE=1 first runs the comparison method into compare-after.json, and,
#   when BASE_CORE names another checkout's packages/core, builds it and runs
#   the same into compare-before.json. TIERS=0 skips the tiers.
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

if [ -n "${TESTS:-}" ]; then
  cargo test --release --locked --no-fail-fast -p vorn-sessiond -p vorn-engine -p vornd >"$out/tests.log" 2>&1 ||
    echo "tests failed" >>"$out/tests.log"
  grep -E '^test result|FAILED|panicked|tests failed' "$out/tests.log" || true
fi

ulimit -n 1048576
ulimit -u unlimited

# One build's comparison: compare BIN_DIR NAME.
compare() {
  echo "comparison method, $2"
  rm -rf /tmp/vorn-compare
  VORN_BENCH_HOST=1 timeout --kill-after=30 3600 "$bin/vorn-scale-bench" compare \
    --sessiond "$1/vorn-sessiond" \
    --vornd "$1/vornd" \
    --work /tmp/vorn-compare \
    --out "$out/compare-$2.json" >"$out/compare-$2.log" 2>&1 ||
    echo "comparison $2 exited with $?" | tee -a "$out/compare-$2.log"
  tail -n 5 "$out/compare-$2.log"
  for p in $(pgrep -x vorn-sessiond; pgrep -x vornd); do
    kill -9 "$p" 2>/dev/null || true
  done
  rm -rf /tmp/vorn-compare
}

if [ -n "${COMPARE:-}" ]; then
  if [ -n "${BASE_CORE:-}" ]; then
    if (cd "$BASE_CORE" && cargo build --release --locked -p vornd -p vorn-sessiond) >"$out/build-base.log" 2>&1; then
      compare "$BASE_CORE/target/release" before
    else
      grep -A 20 '^error' "$out/build-base.log" | head -n 60
    fi
  fi
  compare "$bin" after
fi
[ "${TIERS:-1}" != 0 ] || exit 0
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
  limit=$((900 + tier * 2 / 5))
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
