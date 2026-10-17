#!/bin/sh
# Measures Worktree cleanup as the Wasm component Vorn ships: start, restart,
# calls, events and memory, each repetition in a fresh process.
#   yarn bench:ext-tiers [--reps N] [--out DIR]
set -e
cd "$(dirname "$0")/.."
cargo build --profile ci --locked -p vorn-bench-ext-tiers
exec target/ci/ext-tiers "$@"
